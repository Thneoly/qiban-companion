// Materialize a review sheet; do not infer quality from transport success or model self-report.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const suite=JSON.parse(fs.readFileSync(path.join(__dirname,'../../fixtures/memory-q6-v1.json'),'utf8'));
const output=path.resolve(process.argv[2]||'.cache/q6-review.json');assert(!fs.existsSync(output),'Choose a new review file');
const review={suiteVersion:suite.version,runId:null,model:null,parameters:null,reviewer:null,independentReviewer:null,cases:suite.cases.map(c=>({id:c.id,prompt:c.prompt,expected:c.expected,references:c.expected.references.map(r=>({body:r.body,eventDate:r.eventDate,source:'用户在记忆面板填写'})),rubric:c.review.required,negative:c.review.negative,response:null,transportStatus:'not_run',sourceCorrect:null,omission:null,misuse:null,notes:''})),experience:{reviewer:null,answers:{saved:null,sent:null,cleared:null,cannotRecall:null},confusions:[],acceptable:null},gate:'hold'};
fs.writeFileSync(output,JSON.stringify(review,null,2),{flag:'wx'});console.log('Review sheet created with 50 unreviewed cases. Missing and failed answers remain in the denominator.');
