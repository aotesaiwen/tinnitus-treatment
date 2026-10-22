/* Exercise the shipped DSP with real decoded audio in Chromium, not mock nodes. */
const { chromium } = require('../../app/node_modules/playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
(async () => {
  const base = process.argv[2] || 'http://127.0.0.1:8765/repertoire/';
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage();
    await page.goto(base);
    const results = await page.evaluate(async () => {
      const { createGraph, defaults, pinkBuffer } = await import(new URL('audio-engine.js', location.href));
      const rate = 44100;
      const decoder = new OfflineAudioContext(2, 1, rate);
      const original = await decoder.decodeAudioData(await (await fetch('chopin-nocturne-op9-no2.flac')).arrayBuffer());
      const db = x => 20*Math.log10(Math.max(x, 1e-15));
      function measure(buffer) {
        let peak = 0, sum = 0, finite = true;
        const binSize = Math.round(.1*rate), bins = new Float64Array(Math.ceil(buffer.length/binSize));
        // Independent BS.1770 implementation, checked against the existing render record.
        let k = Math.tan(Math.PI*1681.974450955533/rate), q = .7071752369554196;
        const vh = 10**(3.999843853973347/20), vb = vh**.4996667741545416;
        let a0 = 1+k/q+k*k;
        const b = [(vh+vb*k/q+k*k)/a0, 2*(k*k-vh)/a0, (vh-vb*k/q+k*k)/a0];
        const a = [1, 2*(k*k-1)/a0, (1-k/q+k*k)/a0];
        k = Math.tan(Math.PI*38.13547087602444/rate); q = .5003270373238773; a0 = 1+k/q+k*k;
        const h = [1, 2*(k*k-1)/a0, (1-k/q+k*k)/a0];
        for (let ch = 0; ch < buffer.numberOfChannels; ch++) {
          let z1 = 0, z2 = 0, h1 = 0, h2 = 0;
          const data = buffer.getChannelData(ch);
          for (let i = 0; i < data.length; i++) {
            const x = data[i]; peak = Math.max(peak, Math.abs(x)); sum += x*x; finite &&= Number.isFinite(x);
            const y = b[0]*x+z1; z1 = b[1]*x-a[1]*y+z2; z2 = b[2]*x-a[2]*y;
            const w = y+h1; h1 = -2*y-h[1]*w+h2; h2 = y-h[2]*w;
            bins[Math.floor(i/binSize)] += w*w;
          }
        }
        const blocks = [];
        for (let i = 0; (i+4)*binSize <= buffer.length; i++) blocks.push((bins[i]+bins[i+1]+bins[i+2]+bins[i+3])/(4*binSize));
        const abs = blocks.filter(x => x > 10**((-70+.691)/10));
        const gate = Math.max(10**((-70+.691)/10), abs.reduce((a,b)=>a+b,0)/abs.length*.1);
        const rel = blocks.filter(x => x > gate);
        return { peakDbfs: db(peak), rmsDbfs: db(Math.sqrt(sum/(buffer.length*buffer.numberOfChannels))),
          lufs: -.691+10*Math.log10(rel.reduce((a,b)=>a+b,0)/rel.length), finite };
      }
      async function render(input, settings, changes) {
        const ctx = new OfflineAudioContext(2, input.length, rate);
        const source = ctx.createBufferSource(); source.buffer = input;
        const graph = createGraph(ctx, source);
        graph.apply({ ...defaults, volume: 100, ...settings }, true, true);
        source.start();
        if (changes) await changes(ctx, graph);
        return ctx.startRendering();
      }
      const baseline = measure(original), full = [];
      for (const [bass, brightness, background, mode = 'processed'] of [[0,0,false],[-6,-6,false],[6,6,false],[-6,6,false],[6,-6,false],[2,-2,false],[-2,2,false],[6,6,true],[6,-6,true,'original']]) {
        const out = await render(original, { bass, brightness, background, mode, backgroundVolume:100 });
        const measured = measure(out);
        let maxError = 0;
        if ((!bass && !brightness && !background) || mode === 'original') {
          for (let c=0;c<2;c++) { const a=original.getChannelData(c), b=out.getChannelData(c); for(let i=0;i<a.length;i++) maxError=Math.max(maxError,Math.abs(a[i]-b[i])); }
        }
        // Browser's high-quality 4x resampler checks intersample peaks as well.
        const oversampled = new OfflineAudioContext(2, out.length*4, rate*4);
        const s = oversampled.createBufferSource(); s.buffer=out; s.connect(oversampled.destination); s.start();
        const up = await oversampled.startRendering(); let truePeak=0;
        for(let c=0;c<2;c++) { const data=up.getChannelData(c); for(let i=0;i<data.length;i++) truePeak=Math.max(truePeak, Math.abs(data[i])); }
        full.push({ bass, brightness, background, mode, ...measured, loudnessDifference: measured.lufs-baseline.lufs, maxError, resampledPeakDb:db(truePeak) });
      }
      // Independent sine probes confirm broad low/high shaping in the actual DSP.
      const toneResults = [];
      for (const frequency of [80, 1000, 8000]) {
        const input=decoder.createBuffer(1, rate, rate), data=input.getChannelData(0);
        for(let i=0;i<data.length;i++) data[i]=.1*Math.sin(2*Math.PI*frequency*i/rate);
        for(const [bass, brightness] of [[0,0],[6,0],[0,6]]) {
          const out=await render(input,{bass,brightness}); let sum=0;
          const values=out.getChannelData(0);
          for(let i=rate/2;i<rate;i++) sum+=values[i]*values[i];
          toneResults.push({frequency,bass,brightness,rmsDb:db(Math.sqrt(sum/(rate/2)))});
        }
      }
      // Background lifecycle and repeated switches, with no music to hide leakage.
      const silence=decoder.createBuffer(1, rate*5, rate);
      const ctx=new OfflineAudioContext(2,silence.length,rate), source=ctx.createBufferSource(); source.buffer=silence;
      const graph=createGraph(ctx,source); const active={...defaults,volume:100,background:true,backgroundVolume:100};
      graph.apply(active,true,true); source.start();
      const steps=[[.5,{...active,background:false},true],[1,active,true],[1.5,active,false],[2,active,true],
        [2.5,{...active,mode:'original',bass:6,brightness:6},true],[3,active,true],[3.5,{...active,volume:0},true],[4,{...active,backgroundVolume:0},true]];
      const changes=steps.map(([time,settings,audible])=>ctx.suspend(time).then(()=>{graph.apply(settings,audible);return ctx.resume();}));
      const pending=ctx.startRendering(); await Promise.all(changes); const out=await pending;
      const data=out.getChannelData(0);
      const windows=[];
      for(let start=.15;start<4.9;start+=.5){let peak=0,sum=0;for(let i=Math.round(start*rate);i<Math.round((start+.2)*rate);i++){peak=Math.max(peak,Math.abs(data[i]));sum+=data[i]*data[i];}windows.push({start,peak,rms:Math.sqrt(sum/(.2*rate))});}
      // Stress the safety guard with an intentionally over-full-scale source.
      const hot=decoder.createBuffer(1,rate,rate), hd=hot.getChannelData(0);
      for(let i=0;i<rate;i++)hd[i]=3*Math.sin(2*Math.PI*80*i/rate);
      const overload=measure(await render(hot,{bass:6,brightness:6,background:true,backgroundVolume:100}));
      const noise=pinkBuffer(decoder).getChannelData(0);
      let maxNoiseStep=0; for(let i=1;i<noise.length;i++)maxNoiseStep=Math.max(maxNoiseStep,Math.abs(noise[i]-noise[i-1]));
      // Fast changes during a continuous tone: hold/cancel ramps must stay continuous.
      const dynamic = decoder.createBuffer(1,rate*2,rate);
      const dd=dynamic.getChannelData(0); for(let i=0;i<dd.length;i++)dd[i]=.12*Math.sin(2*Math.PI*220*i/rate);
      const dc=new OfflineAudioContext(2,dynamic.length,rate), ds=dc.createBufferSource(); ds.buffer=dynamic;
      const dg=createGraph(dc,ds); dg.apply({...defaults,volume:100},true,true); ds.start();
      const tasks=[];
      for(let i=1;i<30;i++)tasks.push(dc.suspend(i*.035).then(()=>{dg.apply({...defaults,volume:100,bass:i%2?6:-6,brightness:i%2?-6:6,mode:i%3?'processed':'original'},true);return dc.resume();}));
      const dr=dc.startRendering(); await Promise.all(tasks); const dout=(await dr).getChannelData(0);
      let maxStep=0;for(let i=1;i<dout.length;i++)maxStep=Math.max(maxStep,Math.abs(dout[i]-dout[i-1]));
      return {baseline,full,toneResults,windows,overload,rapidChanges:{maxAdjacentStep:maxStep},noiseLoop:{seamStep:Math.abs(noise[0]-noise[noise.length-1]),maxNoiseStep}};
    });
    assert(Math.abs(results.baseline.lufs+23)<.01);
    for(const r of results.full) {
      assert(r.finite && r.peakDbfs < -.4 && r.resampledPeakDb < 0, 'No digital or intersample clipping');
      if(!r.background) assert(Math.abs(r.loudnessDifference)<.15, `Full-track loudness compensation: ${JSON.stringify(r)}`);
    }
    assert(results.full.at(-1).maxError<1e-6, 'Original ignores all saved processing and background settings');
    assert(results.rapidChanges.maxAdjacentStep<.015, 'Rapid filter/comparison changes remain continuous');
    assert(results.full[0].maxError<1e-6, 'Neutral mode must preserve original waveform');
    const tones=results.toneResults;
    const gain=(freq,bass,brightness)=>tones.find(x=>x.frequency===freq&&x.bass===bass&&x.brightness===brightness).rmsDb-tones.find(x=>x.frequency===freq&&!x.bass&&!x.brightness).rmsDb;
    assert(gain(80,6,0)-gain(8000,6,0)>5.5, 'Bass audibly changes low relative to high frequencies');
    assert(gain(8000,0,6)-gain(80,0,6)>5.5, 'Brightness audibly changes high relative to low frequencies');
    for(const i of [1,3,5,7,8,9]) assert.equal(results.windows[i].peak,0,'Disabled, paused, original and zero-volume background must be silent');
    for(const i of [0,2,4,6]) assert(results.windows[i].rms>.01,'Enabled background must produce sound');
    assert(results.overload.finite&&results.overload.peakDbfs<-.4, 'Overload is bounded');
    assert(results.noiseLoop.seamStep<results.noiseLoop.maxNoiseStep, 'Loop seam is smaller than ordinary noise steps');
    fs.mkdirSync(path.resolve('.runtime/previews'),{recursive:true});
    fs.writeFileSync(path.resolve('.runtime/previews/repertoire-signal.json'),JSON.stringify(results,null,2)+'\n');
    console.log(JSON.stringify(results,null,2));
  } finally {await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
