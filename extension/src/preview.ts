export interface Preview {kind: string; query: string}
const identifier = (table: string, property: string) => `'${table.replaceAll("'", "''")}'[${property.replaceAll("]", "]]")}]`;

/** PBIR projection subset. Reject ambiguous/aggregate/custom visuals rather than invent a query. */
export function visualQuery(definition: any): Preview {
  const visual = definition?.visual;
  const kind: string = visual?.visualType;
  if (!['tableEx','pivotTable','card','cardVisual','barChart','clusteredBarChart','columnChart','clusteredColumnChart','lineChart'].includes(kind)) {
    throw new Error(`Visual '${kind}' is outside basic preview coverage.`);
  }
  const groups = new Set<string>();
  const measures = new Map<string,string>();
  const state = visual?.query?.queryState;
  if (!state || definition.filterConfig || visual.filterConfig) throw new Error('This preview requires explicit projections and no visual filters.');
  for (const role of Object.values(state) as any[]) {
    for (const projection of role?.projections ?? []) {
      const field = projection?.field;
      const value = field?.Column ?? field?.Measure;
      const table = value?.Expression?.SourceRef?.Entity;
      const property = value?.Property;
      if (typeof table !== 'string' || typeof property !== 'string') throw new Error('Unsupported field or aggregation in report visual.');
      const ref = identifier(table, property);
      if (field.Column) groups.add(ref);
      else measures.set(property, ref);
    }
  }
  if (!measures.size) throw new Error('Preview requires at least one explicit measure.');
  const aliases = [...measures].map(([name, ref]) => `"${name.replaceAll('"','""')}", ${ref}`);
  const query = groups.size
    ? `EVALUATE SUMMARIZECOLUMNS(${[...groups, ...aliases].join(', ')})`
    : `EVALUATE ROW(${aliases.join(', ')})`;
  return {kind: kind.includes('line') ? 'line' : /bar|column/i.test(kind) ? 'bar' : /card/i.test(kind) ? 'card' : 'table', query};
}
