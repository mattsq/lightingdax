const fs = require('node:fs');
fs.mkdirSync('dist/vendor', {recursive:true});
for (const [name, file] of [['vega','vega.min.js'],['vega-lite','vega-lite.min.js'],['vega-embed','vega-embed.min.js']]) {
  fs.copyFileSync(`node_modules/${name}/build/${file}`, `dist/vendor/${file}`);
}
for (const name of ['vega','vega-lite','vega-embed','vscode-languageclient','vscode-languageserver-protocol','vscode-languageserver-types','vscode-jsonrpc','semver']) {
  const license = ['LICENSE','LICENSE.txt','License.txt','LICENSE.md'].find(file => fs.existsSync(`node_modules/${name}/${file}`));
  if (license) fs.copyFileSync(`node_modules/${name}/${license}`, `dist/vendor/${name}-LICENSE.txt`);
}
