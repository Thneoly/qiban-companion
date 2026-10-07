import { useEffect, useRef } from 'react';
import { modelSource, type ModelBundle } from './avatarStorage';
import { companionLabels, type CompanionState } from './presentation';
const scripts=new Map<string,Promise<void>>();
function loadScript(url:string) {
  if(scripts.has(url))return scripts.get(url)!;
  const promise=new Promise<void>((resolve,reject)=>{
    const script=document.createElement('script');
    script.src=url;
    script.onload=()=>resolve();
    script.onerror=()=>{script.remove();scripts.delete(url);reject(new Error('Live2D 运行库加载失败'));};
    document.head.appendChild(script);
  });
  scripts.set(url,promise);return promise;
}
export function Live2DRenderer({ active, state, bundle, onError, mouth }: {active:boolean;state:CompanionState;bundle:ModelBundle;onError:()=>void;mouth?:{current:number}}) {
  const canvas=useRef<HTMLCanvasElement>(null);
  const running=useRef(active);
  const presence=useRef(state);
  presence.current=state;
  const mouthRef=useRef(mouth);
  mouthRef.current=mouth;
  useEffect(()=>{running.current=active;},[active]);
  useEffect(()=>{
    let disposed=false;
    let cleanup=()=>{};
    async function mount() {
      try {
        await loadScript('/live2d-runtime/live2dcubismcore.min.js');
        const PIXI=await import('pixi.js');
        const {install}=await import('@pixi/unsafe-eval');
        install({ShaderSystem:PIXI.ShaderSystem});
        type Model=InstanceType<typeof PIXI.Container> & {update(ms:number):void;motion(group:string,index?:number,priority?:number):Promise<boolean>;expression(name:string):Promise<boolean>;anchor:{set(x:number,y:number):void};internalModel?:{coreModel?:{setParameterValueById(id:string,value:number):void}}};
        const host=window as unknown as {PIXI:Record<string,unknown> & {live2d?:{Cubism4ModelSettings:new(source:object)=>{resolveURL:(file:string)=>string};Live2DModel:{from(source:object,options:object):Promise<Model>}}}};
        host.PIXI ??= {...PIXI};
        await loadScript('/live2d-runtime/cubism4.min.js');
        const Live2DModel=host.PIXI.live2d?.Live2DModel;
        if(!Live2DModel)throw Error('Live2D runtime unavailable');
        if(disposed||!canvas.current)return;
        const app=new PIXI.Application({view:canvas.current,width:180,height:260,backgroundAlpha:0,antialias:true,resolution:Math.min(devicePixelRatio,2),autoDensity:true,autoStart:false});
        const source=modelSource(bundle);
        let destroyed=false;
        cleanup=()=>{if(!destroyed){destroyed=true;app.destroy(false,{children:true,texture:true,baseTexture:true});source.dispose();}};
        const settings=new host.PIXI.live2d!.Cubism4ModelSettings(source.settings);
        settings.resolveURL=source.resolveURL;
        const model=await Live2DModel.from(settings,{autoUpdate:false,autoInteract:false});
        if(disposed){model.destroy({texture:true,baseTexture:true});return;}
        if (!Number.isFinite(model.width) || !Number.isFinite(model.height) || model.width <= 0 || model.height <= 0) { model.destroy({texture:true,baseTexture:true}); throw Error('无效的模型尺寸'); }
        model.anchor.set(.5,1);
        model.scale.set(Math.min(180/model.width,260/model.height));
        model.position.set(90,260);
        app.stage.addChild(model);
        app.ticker.maxFPS=30;
        const reduced=matchMedia('(prefers-reduced-motion: reduce)');
        let previous:CompanionState|undefined;
        let mouthWasOpen=false;
        const refs=bundle.settings.FileReferences as {Motions?:Record<string,unknown>;Expressions?:{Name:string}[]};
        app.ticker.add(()=>{
          if(!running.current || document.hidden || reduced.matches)return;
          if(previous!==presence.current){
            previous=presence.current;
            // Optional author-supplied state groups; never invent mouth/audio synchronisation.
            const group=Object.keys(refs.Motions??{}).find(key=>key.toLowerCase()===presence.current);
            const expression=refs.Expressions?.find(item=>item.Name.toLowerCase()===presence.current);
            if(group)void model.motion(group,0,3).catch(()=>{});
            if(expression)void model.expression(expression.Name).catch(()=>{});
          }
          model.update(app.ticker.deltaMS);
          // Speech amplitude overwrites the mouth every frame while speaking;
          // at rest nothing is written, so model motions own the parameter
          // again. The speaking→rest edge writes zero once: a model without
          // idle motions would otherwise keep the last spoken opening forever.
          // Unknown ids are ignored by the core, so models without this
          // parameter simply stay unchanged.
          const open=mouthRef.current?.current ?? 0;
          const core=model.internalModel?.coreModel;
          if(open>0.02){core?.setParameterValueById('ParamMouthOpenY',Math.min(1,open));mouthWasOpen=true;}
          else if(mouthWasOpen){core?.setParameterValueById('ParamMouthOpenY',0);mouthWasOpen=false;}
          app.renderer.render(app.stage);
        });
        model.update(0);app.renderer.render(app.stage);
        app.ticker.remove(app.render,app);
        app.ticker.start();
        canvas.current.dataset.loaded='true';
      } catch {cleanup();cleanup=()=>{};if(!disposed)onError();}
    }
    void mount();
    return ()=>{disposed=true;cleanup();};
  },[bundle,onError]);
  return <><canvas ref={canvas} className="live2d-canvas" aria-label="Live2D 数字人" data-active={active} data-state={state}/><span className="live2d-state" aria-hidden="true">{companionLabels[state]}</span></>;
}
