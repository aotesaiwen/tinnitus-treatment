/* Only opaque sample IDs reach the listening screen. Source metadata is fetched on reveal. */
const $ = (selector) => document.querySelector(selector);
const audio = $('#audio');
const criteria = [
  { id: 'clarity', name: 'Sound clarity', hint: 'Clean tone, without distracting artifacts' },
  { id: 'music', name: 'Musicality', hint: 'Melody, phrasing and a sense of direction' },
  { id: 'replay', name: 'Keep listening', hint: 'How much you want the piece to continue' },
];
const artifactLabels = ['Metallic tones', 'Hiss / distortion', 'Unexpected instruments', 'Mechanical rhythm', 'Abrupt ending'];
let manifest, state, storageKey, sources = null, storageAvailable = true;
let changeToken = 0, restorePosition = 0, resumeOnLoad = false, pendingAction = null;
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const timestamp = (t) => `${Math.floor(Math.max(0,t)/60).toString().padStart(2,'0')}:${Math.floor(Math.max(0,t)%60).toString().padStart(2,'0')}`;
const label = (i) => String.fromCharCode(65+i);
const current = () => manifest.samples.find((s) => s.id === state.order[state.index]);
const answer = (id=state.order[state.index]) => state.answers[id] ??= { ratings:{}, artifacts:[], note:'', played:false, skipped:false };
const complete = (id) => { const a=answer(id); return a.skipped || (a.played && criteria.every((c) => Number.isInteger(a.ratings[c.id]) && a.ratings[c.id]>=1 && a.ratings[c.id]<=5)); };

function shuffle(values) {
  const out = [...values];
  for (let i=out.length-1;i>0;i--) {
    const n=i+1, limit=Math.floor(4294967296/n)*n;
    let r; do { r=crypto.getRandomValues(new Uint32Array(1))[0]; } while(r>=limit);
    const j=r%n; [out[i],out[j]]=[out[j],out[i]];
  }
  return out;
}
function freshState() {
  return { version:manifest.version, session:crypto.randomUUID(), createdAt:new Date().toISOString(),
    order:shuffle(manifest.samples.map((s)=>s.id)), index:0, answers:{}, volume:.65, keepPosition:true, revealed:false };
}
function persist() {
  try { localStorage.setItem(storageKey,JSON.stringify(state)); }
  catch { storageAvailable=false; }
}
function loadState() {
  try {
    const s=JSON.parse(localStorage.getItem(storageKey));
    const ids=manifest.samples.map(x=>x.id);
    if(s && s.version===manifest.version && Array.isArray(s.order) && s.order.length===ids.length &&
       new Set(s.order).size===ids.length && s.order.every(x=>ids.includes(x)) &&
       Number.isInteger(s.index) && s.index>=0 && s.index<ids.length && s.answers && typeof s.answers==='object') {
      s.volume=Number.isFinite(s.volume)?Math.max(0,Math.min(1,s.volume)):.65;
      for (const id of s.order) {
        const a=s.answers[id];
        if(!a || !a.ratings || typeof a.ratings!=='object' || !Array.isArray(a.artifacts) || typeof a.note!=='string') delete s.answers[id];
      }
      return s;
    }
  } catch { /* A malformed or stale local session must not break playback. */ }
  return freshState();
}
function renderNav() {
  $('#sample-list').innerHTML=state.order.map((id,i)=>{
    const a=answer(id), ready=complete(id);
    const name=state.revealed && sources? sources.samples[id]?.shortLabel || `Sample ${label(i)}` : `Sample ${label(i)}`;
    const status=a.skipped?'–':ready?'✓':a.played?'·':'';
    return `<button type="button" class="sample-item ${i===state.index?'active':''}" data-index="${i}" aria-current="${i===state.index?'true':'false'}" aria-label="Sample ${label(i)}${a.skipped?', skipped':ready?', rated':''}"><span class="sample-badge">${label(i)}</span><span class="sample-item-name">${esc(name)}</span><span class="sample-state" aria-hidden="true">${status}</span></button>`;
  }).join('');
  const count=state.order.filter(complete).length;
  $('#progress-count').textContent=`${count} / ${state.order.length}`;
  $('#reveal').disabled=count!==state.order.length || state.revealed;
  $('#finish-hint').textContent=count===state.order.length?'Your impressions are saved. Ready to see what made each sound?':`${state.order.length-count} sample${state.order.length-count===1?'':'s'} left to rate or skip. Your progress is saved automatically.`;
  $('#finish-panel').hidden=state.revealed;
}
function renderAnswer() {
  const a=answer();
  $('#ratings').innerHTML=criteria.map((c)=>`<fieldset class="rating-row" ${state.revealed?'disabled':''}><legend>${c.name}<span class="rating-subtitle">${c.hint}</span></legend><div class="rating-choices">${[1,2,3,4,5].map((n)=>`<label class="rating-choice"><input type="radio" name="${c.id}" value="${n}" ${a.ratings[c.id]===n?'checked':''} aria-label="${c.name}: ${n} out of 5"><span>${n}</span></label>`).join('')}</div></fieldset>`).join('');
  $('#artifact-options').innerHTML=artifactLabels.map((text)=>`<label class="artifact-option"><input type="checkbox" value="${esc(text)}" ${a.artifacts.includes(text)?'checked':''} ${state.revealed?'disabled':''}><span>${esc(text)}</span></label>`).join('');
  $('#notes').value=a.note; $('#notes').disabled=state.revealed;
  $('#skip').hidden=state.revealed;
  $('#skip').textContent=a.skipped?'Undo skip — rate this sample':'Skip — I cannot judge this sample';
  $('#next').textContent=state.index===state.order.length-1?'Back to sample A →':'Next sample →';
  updateSaveStatus();
}
function updateSaveStatus() {
  const a=answer();
  $('#save-status').textContent=!storageAvailable?'Browser storage is unavailable. Export your ratings before closing.':state.revealed?'Original ratings locked.':a.skipped?'Skipped. You can undo this before revealing.':!a.played?'Play this sample to begin.':complete(state.order[state.index])?'Saved. Move on whenever you are ready.':'Choose a score for all three impressions.';
}
function syncTransport() {
  const playing=!audio.paused && !audio.ended;
  $('.player-panel').classList.toggle('playing',playing);
  $('#play').innerHTML=`${playing?'Pause':'Play sample'} <span class="play-symbol" aria-hidden="true"></span>`;
  $('#play-state').textContent=playing?(audio.readyState<3?'LOADING AUDIO…':'LISTENING'):audio.ended?'TAKE YOUR TIME':audio.currentTime>0?'PAUSED':'READY WHEN YOU ARE';
}
function select(index, initial=false) {
  if(index<0 || index>=state.order.length) return;
  const wasPlaying=!audio.paused && !audio.ended;
  const at=Number.isFinite(audio.currentTime)?audio.currentTime:0;
  audio.pause(); state.index=index; persist();
  const s=current(); changeToken++;
  restorePosition=!initial && state.keepPosition?Math.min(at,Math.max(0,s.duration-0.1)):0;
  resumeOnLoad=!initial && wasPlaying;
  $('#sample-heading').textContent=state.revealed && sources?sources.samples[s.id]?.label || `Sample ${label(index)}`:`Sample ${label(index)}`;
  $('#sample-letter').textContent=label(index);
  $('#sample-position').textContent=`SAMPLE ${(index+1).toString().padStart(2,'0')} / ${state.order.length.toString().padStart(2,'0')}`;
  $('#player-error').hidden=true; $('#play').disabled=false;
  $('#seek').max=s.duration; $('#seek').value=restorePosition;
  $('#time-current').textContent=timestamp(restorePosition); $('#time-total').textContent=timestamp(s.duration);
  audio.src=s.audio; audio.load();
  renderNav(); renderAnswer(); syncTransport();
}
async function playPause() {
  if(!audio.paused){audio.pause();return;}
  $('#player-error').hidden=true;
  try { await audio.play(); }
  catch { $('#player-error').textContent='This sample could not play. Please try again or reload the page.'; $('#player-error').hidden=false; }
}
audio.addEventListener('loadedmetadata',async()=>{
  const token=changeToken;
  if(Number.isFinite(audio.duration)) {
    audio.currentTime=Math.min(restorePosition,Math.max(0,audio.duration-.05));
    $('#seek').max=audio.duration; $('#time-total').textContent=timestamp(audio.duration);
  }
  if(resumeOnLoad) { resumeOnLoad=false; try{await audio.play();}catch{if(token===changeToken)syncTransport();} }
});
audio.addEventListener('play',()=>{if(!state.revealed){answer().played=true;persist();renderNav();updateSaveStatus();}syncTransport();});
audio.addEventListener('pause',syncTransport);audio.addEventListener('ended',syncTransport);
audio.addEventListener('waiting',syncTransport);audio.addEventListener('playing',syncTransport);
audio.addEventListener('timeupdate',()=>{$('#seek').value=audio.currentTime;$('#time-current').textContent=timestamp(audio.currentTime);});
audio.addEventListener('error',()=>{if(!manifest)return;$('#player-error').textContent='Audio could not be loaded. Try another sample, then return here. You can skip a sample that remains unavailable.';$('#player-error').hidden=false;syncTransport();});
$('#play').addEventListener('click',playPause);
$('#restart').addEventListener('click',()=>{audio.currentTime=0;if(audio.paused)playPause();});
$('#seek').addEventListener('input',()=>{if(Number.isFinite(audio.duration))audio.currentTime=Number($('#seek').value);});
$('#volume').addEventListener('input',()=>{audio.volume=state.volume=Number($('#volume').value);persist();});
$('#keep-position').addEventListener('change',()=>{state.keepPosition=$('#keep-position').checked;persist();});
$('#sample-list').addEventListener('click',(e)=>{const b=e.target.closest('[data-index]');if(b)select(Number(b.dataset.index));});
$('#next').addEventListener('click',()=>{select((state.index+1)%state.order.length);$('.player-panel').scrollIntoView({behavior:matchMedia('(prefers-reduced-motion: reduce)').matches?'instant':'smooth',block:'nearest'});});
$('#ratings').addEventListener('change',(e)=>{if(state.revealed || !e.target.matches('input[type=radio]'))return;answer().ratings[e.target.name]=Number(e.target.value);answer().skipped=false;persist();renderNav();updateSaveStatus();$('#skip').textContent='Skip — I cannot judge this sample';});
$('#artifact-options').addEventListener('change',()=>{if(state.revealed)return;answer().artifacts=[...$('#artifact-options').querySelectorAll('input:checked')].map(x=>x.value);persist();updateSaveStatus();});
$('#notes').addEventListener('input',()=>{if(state.revealed)return;answer().note=$('#notes').value;persist();updateSaveStatus();});
$('#skip').addEventListener('click',()=>{answer().skipped=!answer().skipped;persist();renderNav();renderAnswer();});
document.addEventListener('keydown',(e)=>{if(!manifest || !state || e.target.closest('input,textarea,button,a,dialog,select'))return;if(e.code==='Space'){e.preventDefault();playPause();}});

function askConfirmation(action) {
  pendingAction=action;
  const reset=action==='reset';
  $('#confirm-title').textContent=reset?'Start another round?':'Reveal the sources?';
  $('#confirm-text').textContent=reset?'This clears the current ratings. Export them first if you want to keep them. The excerpts will be the same in a new order; knowing their sources cannot be undone.':'Your ratings will be locked for this round. You can continue listening after the reveal.';
  $('#confirm-action').textContent=reset?'Start new round':'Reveal results';
  $('#confirm-dialog').showModal();
}
$('#reveal').addEventListener('click',()=>askConfirmation('reveal'));
$('#new-session').addEventListener('click',()=>askConfirmation('reset'));
$('#confirm-dialog').addEventListener('close',async()=>{
  if($('#confirm-dialog').returnValue!=='confirm')return;
  if(pendingAction==='reset') {
    audio.pause();state=freshState();sources=null;persist();audio.volume=state.volume;$('#volume').value=state.volume;$('#keep-position').checked=state.keepPosition;document.body.classList.remove('revealed');$('#results').hidden=true;select(0,true);$('#finish-panel').hidden=false;return;
  }
  $('#reveal').disabled=true;
  try {
    const r=await fetch('reveal.json',{cache:'no-store'});if(!r.ok)throw new Error();
    const data=await r.json();
    if(data.version!==manifest.version || !state.order.every(id=>data.samples[id]))throw new Error();
    sources=data;state.revealed=true;state.revealedAt=new Date().toISOString();persist();renderResults();renderNav();renderAnswer();
    $('#sample-heading').textContent=sources.samples[current().id].label;
    $('#results').scrollIntoView({behavior:matchMedia('(prefers-reduced-motion: reduce)').matches?'instant':'smooth'});
  } catch {$('#global-error').textContent='The source information could not be loaded. Your ratings are safe; please try revealing again.';$('#global-error').hidden=false;renderNav();}
});
function renderResults() {
  document.body.classList.add('revealed');$('#results').hidden=false;$('#global-error').hidden=true;
  const rows=state.order.map((id,i)=>({id,index:i,a:answer(id),source:sources.samples[id]}));
  rows.sort((x,y)=>(y.a.skipped?-1:y.a.ratings.replay||0)-(x.a.skipped?-1:x.a.ratings.replay||0));
  $('#results-list').innerHTML=rows.map(({index,a,source})=>`<article class="result-row"><div class="result-letter">${label(index)}</div><div><h3 class="result-source">${esc(source.label)}</h3><p class="result-detail">${esc(source.description)}</p><p class="result-detail">${esc(source.configuration || '')}</p>${source.url?`<p class="result-detail"><a href="${esc(source.url)}" target="_blank" rel="noopener noreferrer">Source & documentation ↗</a></p>`:''}${a.note?`<p class="result-note">${esc(a.note)}</p>`:''}${a.artifacts.length?`<p class="result-detail">Noted: ${a.artifacts.map(esc).join(' · ')}</p>`:''}</div><div class="result-score">${a.skipped?'Skipped':criteria.map(c=>`${c.name}<b>${a.ratings[c.id]??'–'} / 5</b><br>`).join('')}</div></article>`).join('');
}
$('#export').addEventListener('click',()=>{
  const result={schema:'listening-room-feedback/v1',dataset:manifest.version,session:state.session,createdAt:state.createdAt,revealedAt:state.revealedAt,method:manifest.method,samples:state.order.map((id,i)=>({anonymousLabel:label(i),id,...answer(id),source:sources?.samples[id]??null}))};
  const url=URL.createObjectURL(new Blob([JSON.stringify(result,null,2)],{type:'application/json'}));
  const a=document.createElement('a');a.href=url;a.download=`listening-room-${state.session.slice(0,8)}.json`;a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);
});
function showCoverage() {
  $('#coverage-content').innerHTML=`<p>${esc(manifest.method)}</p><p>${esc(manifest.coverageNote)}</p>${manifest.coverage.map(c=>`<div class="coverage-row"><strong>${esc(c.name)}</strong><span class="availability">${esc(c.status)}</span><p>${esc(c.detail)}</p></div>`).join('')}<p>Only genuinely generated or explicitly identified reference excerpts enter the listening set. No unavailable model is represented by another model's audio.</p><p>Audio files use anonymous filenames. Source metadata is fetched only when you reveal. This is a personal listening demo, not protection against deliberately inspecting the site's files.</p><p>${esc(manifest.attribution || '')}</p>`;
  $('#coverage-dialog').showModal();
}
document.querySelectorAll('.coverage-trigger').forEach(b=>b.addEventListener('click',showCoverage));
$('#close-coverage').addEventListener('click',()=>$('#coverage-dialog').close());

async function init() {
  try {
    const r=await fetch('manifest.json',{cache:'no-store'});if(!r.ok)throw new Error();manifest=await r.json();
    if(!Array.isArray(manifest.samples) || manifest.samples.length===0){$('#session-meta').textContent='AUDIO GENERATION IN PROGRESS';$('#global-error').textContent='The listening room is ready. The real model outputs are still being prepared; no placeholder audio is being presented.';$('#global-error').hidden=false;return;}
    storageKey=`listening-room:${manifest.version}`;state=loadState();persist();
    audio.volume=state.volume;$('#volume').value=state.volume;$('#keep-position').checked=state.keepPosition;
    $('#session-meta').textContent=`${manifest.samples.length} EXCERPTS · ${manifest.samples[0].duration}s EACH · LEVEL MATCHED`;
    $('#listening').hidden=false;$('#finish-panel').hidden=false;select(state.index,true);
    if(state.revealed) {
      try {const rr=await fetch('reveal.json',{cache:'no-store'});if(!rr.ok)throw new Error();const data=await rr.json();if(data.version!==manifest.version)throw new Error();sources=data;renderResults();renderNav();$('#sample-heading').textContent=sources.samples[current().id].label;}
      catch{$('#global-error').textContent='Your saved ratings are locked, but source details could not load. Reload to restore the reveal.';$('#global-error').hidden=false;}
    }
  } catch {$('#global-error').textContent='The listening set could not be loaded. Please reload the page.';$('#global-error').hidden=false;$('#session-meta').textContent='SESSION UNAVAILABLE';}
}
init();
