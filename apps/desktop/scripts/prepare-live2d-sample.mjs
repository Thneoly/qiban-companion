// Internal integration sample only. See docs/development/model-settings-live2d.md before distribution.
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const here=path.dirname(fileURLToPath(import.meta.url));
const root=path.resolve(here,'../public/live2d-local');
const manifest=JSON.parse(await readFile(path.join(here,'live2d-sample-manifest.json'),'utf8'));
function verify(bytes,item) {
  if(item.sha256)return createHash('sha256').update(bytes).digest('hex')===item.sha256;
  return createHash('sha1').update(Buffer.from('blob '+bytes.length+'\0')).update(bytes).digest('hex')===item.gitBlob;
}
for(let i=0;i<manifest.length;i+=3) {
  await Promise.all(manifest.slice(i,i+3).map(async item=>{
    const target=path.resolve(root,item.file);
    if(!target.startsWith(root+path.sep))throw Error('Invalid asset path');
    let existing;
    try {existing=await readFile(target);}catch(error){if(error.code!=='ENOENT')throw error;}
    if(existing){if(!verify(existing,item))throw Error('Existing asset differs: '+item.file);return;}
    const response=await fetch(item.url,{signal:AbortSignal.timeout(30000)});
    if(!response.ok)throw Error('Download failed: '+item.file+' '+response.status);
    const bytes=Buffer.from(await response.arrayBuffer());
    if(bytes.length>16*1024*1024||!verify(bytes,item))throw Error('Asset verification failed: '+item.file);
    await mkdir(path.dirname(target),{recursive:true});
    await writeFile(target,bytes,{flag:'wx'});
  }));
}
console.log('Verified '+manifest.length+' local Live2D sample files. Internal test assets, not a commercial character license.');
