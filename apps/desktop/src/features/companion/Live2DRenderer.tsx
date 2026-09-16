import { useEffect, useRef } from 'react';
const scripts=new Map<string,Promise<void>>();
function loadScript(url:string) {
  if(scripts.has(url))return scripts.get(url)!;
  const promise=new Promise<void>((resolve,reject)=>{
    const script=document.createElement('script');
    script.src=url;
    script.onload=()=>resolve();
    script.onerror=()=>{script.remove();scripts.delete(url);reject(new Error('缺少本机Live2D测试资源'));};
    document.head.appendChild(script);
  });
  scripts.set(url,promise);return promise;
}
export function Live2DRenderer({ active, onError }: {active:boolean;onError:()=>void}) {
  const canvas=useRef<HTMLCanvasElement>(null);
  const running=useRef(active);
  useEffect(()=>{running.current=active;},[active]);
  useEffect(()=>{
    let disposed=false;
    let cleanup=()=>{};
    async function mount() {
      try {
        await loadScript('/live2d-local/live2dcubismcore.min.js');
        const PIXI=await import('pixi.js');
        const {install}=await import('@pixi/unsafe-eval');
        install({ShaderSystem:PIXI.ShaderSystem});
        type Model=InstanceType<typeof PIXI.Container> & {update(ms:number):void;anchor:{set(x:number,y:number):void}};
        const host=window as unknown as {PIXI:Record<string,unknown> & {live2d?:{Live2DModel:{from(url:string,options:object):Promise<Model>}}}};
        host.PIXI ??= {...PIXI};
        await loadScript('/live2d-local/cubism4.min.js');
        const Live2DModel=host.PIXI.live2d?.Live2DModel;
        if(!Live2DModel)throw Error('Live2D runtime unavailable');
        if(disposed||!canvas.current)return;
        const app=new PIXI.Application({view:canvas.current,width:180,height:205,backgroundAlpha:0,antialias:true,resolution:Math.min(devicePixelRatio,2),autoDensity:true,autoStart:false});
        let destroyed=false;
        cleanup=()=>{if(!destroyed){destroyed=true;app.destroy(false,{children:true,texture:true,baseTexture:true});}};
        const model=await Live2DModel.from('/live2d-local/Hiyori/Hiyori.model3.json',{autoUpdate:false,autoInteract:false});
        if(disposed){model.destroy({texture:true,baseTexture:true});return;}
        model.anchor.set(.5,1);
        model.scale.set(Math.min(180/model.width,205/model.height));
        model.position.set(90,205);
        app.stage.addChild(model);
        app.ticker.maxFPS=30;
        app.ticker.add(()=>{
          if(running.current && !document.hidden){model.update(app.ticker.deltaMS);app.renderer.render(app.stage);}
        });
        app.ticker.remove(app.render,app);
        app.ticker.start();
        canvas.current.dataset.loaded='true';
      } catch {cleanup();cleanup=()=>{};if(!disposed)onError();}
    }
    void mount();
    return ()=>{disposed=true;cleanup();};
  },[onError]);
  return <canvas ref={canvas} className="live2d-canvas" aria-label="Live2D实验角色" data-active={active}/>;
}
