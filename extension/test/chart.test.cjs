const {test}=require('node:test');const assert=require('node:assert/strict');
test('bar and line render through the CSP-compatible interpreter',async()=>{
  const [vega,vl,{expressionInterpreter}]=await Promise.all([import('vega'),import('vega-lite'),import('vega-interpreter')]);
  for(const mark of ['bar','line']){
    const spec=vl.compile({data:{values:[{category:'A',value:2},{category:'B',value:3}]},mark,encoding:{x:{field:'category',type:'nominal'},y:{field:'value',type:'quantitative'}}}).spec;
    const view=new vega.View(vega.parse(spec,null,{ast:true}),{expr:expressionInterpreter,renderer:'none'});
    try{const svg=await view.toSVG();assert.match(svg,/<svg/);assert.match(svg,/>A</);}finally{view.finalize();}
  }
});
