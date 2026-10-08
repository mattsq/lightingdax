import * as vscode from 'vscode';
import {randomBytes} from 'node:crypto';
import {LanguageClient, LanguageClientOptions, ServerOptions} from 'vscode-languageclient/node';
import {visualQuery} from './preview';

let client: LanguageClient | undefined;
let panel: vscode.WebviewPanel | undefined;
let extensionUri: vscode.Uri;
let timer: NodeJS.Timeout | undefined;
let running = false;
let pending: vscode.TextDocument | undefined;
let kind = 'table';
let generation = 0;
const bindings = new Map<string, {table: string; name: string}>();

async function start(): Promise<void> {
  const config = vscode.workspace.getConfiguration('lightingdax');
  const projectPath = config.get<string>('projectPath') || vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
  const server: ServerOptions = {command: config.get<string>('serverPath') || 'lightingdax', args: ['lsp']};
  const options: LanguageClientOptions = {
    documentSelector: [{language:'dax'}],
    initializationOptions: {projectPath, adapterProgram: config.get('adapterProgram'), adapterArgs: config.get('adapterArgs')},
  };
  client = new LanguageClient('lightingdax', 'Lightning DAX', server, options);
  await client.start();
}
function getClient(): LanguageClient {if (!client) throw new Error('DAX server has not started.');return client;}
async function command(action: () => Promise<unknown>): Promise<void> {
  try {await action();} catch (error) {void vscode.window.showErrorMessage(String(error));}
}
export async function activate(context: vscode.ExtensionContext): Promise<void> {
  extensionUri = context.extensionUri;
  const register = (name: string, action: () => Promise<unknown>) => context.subscriptions.push(vscode.commands.registerCommand(name, () => command(action)));
  register('lightingdax.restart', async () => {
    generation++; pending = undefined; if (timer) clearTimeout(timer); await client?.stop(); client = undefined;
    await start();
    for (const document of vscode.workspace.textDocuments) {
      const binding = bindings.get(document.uri.toString());
      if (binding) await getClient().sendRequest('lightingdax/bindMeasure', {uri:document.uri.toString(), ...binding});
    }
  });
  register('lightingdax.openMeasure', async () => {
    const measures = await getClient().sendRequest<Array<{table:string;name:string;expression:string}>>('lightingdax/measures');
    const selected = await vscode.window.showQuickPick(measures.map(m => ({label:m.name, description:m.table, measure:m})), {placeHolder:'Choose a model measure'});
    if (!selected) return;
    const m = selected.measure;
    const existing = vscode.workspace.textDocuments.find(d => {
      const binding = bindings.get(d.uri.toString());
      return binding?.table === m.table && binding?.name === m.name;
    });
    if (existing) {await vscode.window.showTextDocument(existing);return;}
    // Untitled DAX buffers are editable; explicit Save Measure writes back to TMDL.
    const document = await vscode.workspace.openTextDocument({language:'dax', content:m.expression});
    await vscode.window.showTextDocument(document);
    const binding = {table:m.table, name:m.name};
    await getClient().sendRequest('lightingdax/bindMeasure', {uri:document.uri.toString(), ...binding});
    bindings.set(document.uri.toString(), binding);
    kind = 'card'; await evaluate(document);
  });
  register('lightingdax.saveMeasure', async () => {
    const document = vscode.window.activeTextEditor?.document;
    if (!document || !bindings.has(document.uri.toString())) throw new Error('Use DAX: Open Model Measure first.');
    await getClient().sendRequest('lightingdax/saveMeasure', {uri:document.uri.toString()});
    void vscode.window.showInformationMessage('Measure saved to TMDL.');
  });
  register('lightingdax.evaluate', async () => {
    const document = vscode.window.activeTextEditor?.document;
    if (!document || document.languageId !== 'dax') throw new Error('Open a DAX document first.');
    await evaluate(document);
  });
  register('lightingdax.preview', async () => {
    const visuals = await getClient().sendRequest<Array<{path:string;kind:string;definition:unknown}>>('lightingdax/visuals');
    const selected = await vscode.window.showQuickPick((visuals || []).map(v => ({label:v.kind,description:v.path,visual:v})), {placeHolder:'Choose a report visual'});
    if (!selected) return;
    const preview = visualQuery(selected.visual.definition);
    const document = await vscode.workspace.openTextDocument({language:'dax', content:preview.query});
    await vscode.window.showTextDocument(document); kind = preview.kind; await evaluate(document);
  });
  context.subscriptions.push(vscode.workspace.onDidChangeTextDocument(event => {
    if (event.document.languageId !== 'dax' || !event.contentChanges.length || !vscode.workspace.getConfiguration('lightingdax').get('liveEvaluation')) return;
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => void evaluate(event.document).catch(error => render({source:'none',warning:String(error)})), 300);
  }));
  context.subscriptions.push(vscode.workspace.onDidCloseTextDocument(document => bindings.delete(document.uri.toString())));
  context.subscriptions.push({dispose: () => {if (timer) clearTimeout(timer);panel?.dispose();}});
  await start();
}
async function evaluate(document: vscode.TextDocument): Promise<void> {
  pending = document;
  if (running) return;
  running = true;
  try {
    while (pending) {
      const current = pending; pending = undefined; const version = current.version; const epoch = generation;
      render({source:'none', warning:'Evaluating…'});
      try {
        const response = await getClient().sendRequest<{version:number;evaluation:unknown}>('lightingdax/evaluate', {uri:current.uri.toString()});
        if (epoch === generation && current.version === version && response.version === version && !pending) render(response.evaluation);
      } catch (error) {if (epoch === generation && current.version === version && !pending) render({source:'none',warning:String(error)});}
    }
  } finally {running = false;}
}
function render(evaluation: unknown): void {
  if (!panel) {
    panel = vscode.window.createWebviewPanel('lightingdax.results','DAX Results',vscode.ViewColumn.Beside,{enableScripts:true,localResourceRoots:[vscode.Uri.joinPath(extensionUri,'dist','vendor')]});
    panel.onDidDispose(() => {panel = undefined;});
    const nonce = randomBytes(16).toString('hex');
    const scripts = ['vega.min.js','vega-lite.min.js','vega-embed.min.js'].map(name => `<script nonce="${nonce}" src="${panel!.webview.asWebviewUri(vscode.Uri.joinPath(extensionUri,'dist','vendor',name))}"></script>`).join('');
    panel.webview.html = `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'nonce-${nonce}' ${panel.webview.cspSource}; style-src 'unsafe-inline'; img-src data: ${panel.webview.cspSource};"><style>body{font-family:var(--vscode-font-family);color:var(--vscode-foreground);padding:20px}h1{font-size:20px}table{border-collapse:collapse;width:100%;margin-top:16px}th,td{text-align:left;padding:8px;border-bottom:1px solid var(--vscode-panel-border)}#source{font-weight:600}#warning{padding:16px 0}#card{font-size:36px;padding:20px 0}button,select{margin:8px;background:var(--vscode-button-background);color:var(--vscode-button-foreground);border:0;padding:6px}#chart{background:white}</style></head><body><h1>DAX Results</h1><div id="source"></div><div id="warning"></div><select id="kind"><option value="table">Table / matrix</option><option value="card">Card</option><option value="bar">Bar</option><option value="line">Line</option></select><div id="card"></div><div id="chart"></div><div id="data"></div><button id="prev">Previous</button><span id="page"></span><button id="next">Next</button>${scripts}<script nonce="${nonce}">
      const vscode=acquireVsCodeApi();let state, page=0, view;
      const el=id=>document.getElementById(id);
      const value=c=>c.type==='blank'?'BLANK':String(c.value);
      function show(){
        const e=state?.evaluation||{};el('source').textContent='Source: '+e.source+(e.status?' • '+e.status:'');el('warning').textContent=e.warning||'';
        el('data').replaceChildren();el('card').textContent='';el('chart').replaceChildren();if(view){view.finalize();view=undefined;}
        const table=e.result;const rows=table?.rows||[];const columns=table?.columns||[];const kind=el('kind').value;
        for(const id of ['kind','prev','next','page'])el(id).hidden=!table;
        if(!table)return;
        el('prev').disabled=page===0;el('next').disabled=(page+1)*100>=rows.length;el('page').textContent=rows.length+' rows • page '+(page+1);
        if(kind==='card'){el('card').textContent=rows[0]?.[0]?value(rows[0][0]):'No rows';return;}
        if((kind==='bar'||kind==='line')&&columns.length>=2){
          if(rows.some(row=>row[1]?.type==='integer'&&!Number.isSafeInteger(Number(row[1].value)))){el('warning').textContent='This chart would round a large integer. Use the table or card for its exact value.';return;}
          const data=rows.map(row=>({category:row[0]?.type==='blank'?null:row[0]?.value,value:row[1]?.type==='blank'?null:row[1]?.value}));
          vegaEmbed(el('chart'),{data:{values:data},mark:kind,encoding:{x:{field:'category',title:columns[0],type:rows[0]?.[0]?.type==='datetime'?'temporal':'nominal'},y:{field:'value',title:columns[1],type:'quantitative'}},width:480},{actions:false,renderer:'svg',ast:true}).then(r=>{view=r.view;}).catch(err=>{el('warning').textContent=String(err);});return;
        }
        const node=document.createElement('table'),head=document.createElement('tr');for(const name of columns){const th=document.createElement('th');th.textContent=name;head.append(th);}node.append(head);
        for(const row of rows.slice(page*100,(page+1)*100)){const tr=document.createElement('tr');for(const cell of row){const td=document.createElement('td');td.textContent=value(cell);tr.append(td);}node.append(tr);}el('data').append(node);
      }
      el('kind').onchange=()=>{page=0;show();};el('prev').onclick=()=>{page--;show();};el('next').onclick=()=>{page++;show();};
      window.addEventListener('message',event=>{state=event.data;page=0;el('kind').value=state.kind;show();});vscode.postMessage({ready:true});
    </script></body></html>`;
    panel.webview.onDidReceiveMessage(message => {if (message.ready) void panel?.webview.postMessage(last);});
  }
  last = {evaluation,kind}; void panel.webview.postMessage(last);
}
let last: unknown;
export async function deactivate(): Promise<void> {generation++;if (timer) clearTimeout(timer);await client?.stop();}
