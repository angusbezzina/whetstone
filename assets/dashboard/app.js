(()=>{"use strict";
const d=document,$=i=>d.getElementById(i),TK="wh-theme";
const theme=t=>{t=t||"system";t==="system"?d.documentElement.removeAttribute("data-theme"):d.documentElement.setAttribute("data-theme",t);try{localStorage.setItem(TK,t)}catch{}for(const b of d.querySelectorAll(".theme button"))b.setAttribute("aria-pressed",String(b.dataset.theme===t))};
let saved="system";try{saved=localStorage.getItem(TK)||"system"}catch{}theme(saved);
const h=(t,a,...c)=>{const e=d.createElement(t);for(const[k,v]of Object.entries(a||{}))if(v!=null&&v!==false)k==="text"?e.textContent=v:k==="class"?e.className=v:k.startsWith("on")?e.addEventListener(k.slice(2),v):e.setAttribute(k,v===true?"":v);e.append(...c.flat(9).filter(x=>x!=null&&x!==false));return e};
const st=l=>l&&h("span",{class:"state "+l.tone,text:l.label});
const date=v=>{const t=new Date(v);return isNaN(t)?"":t.toLocaleDateString(void 0,{day:"numeric",month:"short",year:"numeric"})};
const time=v=>{const t=new Date(v);return isNaN(t)?"":t.toLocaleTimeString(void 0,{hour:"2-digit",minute:"2-digit"})};
const W={value:null,journal:[],views:null,edit:null};
const csrf=()=>sessionStorage.getItem("whetstone_csrf");
const say=t=>{const a=$("announce");a.textContent="";setTimeout(()=>a.textContent=t,30)};
async function json(r){const j=await r.json();if(!r.ok)throw Error(j.summary||j.reason_code||"Request failed ("+r.status+")");return j}
async function inspect(q){return json(await fetch("/api/inspect",q?{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify(q)}:void 0))}
let edited=false;
async function cmd(body){const t=csrf();if(!t)throw Error("Editing is not available in this session.");if(!edited){await json(await fetch("/session/edit",{method:"POST",headers:{"X-Whetstone-CSRF":t}}));edited=true}return json(await fetch("/api/command",{method:"POST",headers:{"Content-Type":"application/json","X-Whetstone-CSRF":t},body:JSON.stringify(body)}))}
const tabs=()=>[...d.querySelectorAll('[role="tab"]')];
const GATED=["tab-checks","tab-changelog","tab-requests"];
function header(c){$("project").textContent=c?.header.project||"";$("agreement-state").textContent=c?.header.agreement||"unavailable";const n=c?.header.drafts||0;$("draft-count").textContent=n;$("draft-word").textContent=n===1?"draft":"drafts";const r=(c?.requests||[]).filter(x=>!x.answer).length,b=$("req-count");b.hidden=!r;b.textContent=r||"";b.setAttribute("aria-label",r+" open");for(const t of tabs()){const g=!c?.established&&GATED.includes(t.id);t.setAttribute("aria-disabled",String(g));t.title=g?"Available once the agreement has a mission and a rule":""}}
function steps(c){return h("ol",{class:"steps"},c.onboarding.steps.map((x,i)=>h("li",{"data-step":x.key},h("b",{text:String(i+1)}),h("span",{},h("strong",{text:x.label}),x.hint),st(x.done?{tone:"pass",label:"done"}:x.optional?{tone:"muted",label:"optional"}:{tone:"warn",label:"to do"}))))}
function tools(c){const t=c.onboarding.tools.filter(x=>x.state!=="ok");return t.length?h("div",{class:"rows"},t.map(x=>row(x.tool,[x.detail,x.fix?[" ",h("code",{text:x.fix})]:null],{tone:"warn",label:x.state}))):null}
function onboarding(c,el){const edit=!!csrf(),p=h("section",{class:"onboard",id:"onboard"},h("h1",{text:"This project has no agreement yet."}),h("p",{text:"State the mission, pick the principles you hold, point at a codebase you admire, and accept the rules that follow. Every agent and person is then briefed on those rules and checked against them."}),steps(c),tools(c),h("div",{class:"act"},edit?h("button",{class:"btn primary",type:"button",id:"start-onboarding",text:"Start onboarding",onclick:()=>editor().then(m=>m.onboard(api,p))}):null,h("span",{class:"or",text:edit?"or from the terminal":"From the terminal"}),h("code",{class:"cmd",text:c.onboarding.command})),h("p",{class:"private",text:"Nothing is shared, installed or published by this step. Existing lint, tests and CI are inspected, not replaced."}));el.append(p)}
function panel(title,count,...body){return h("section",{class:"panel"},h("div",{class:"ph"},h("h2",{text:title}),count==null?null:h("span",{class:"count",text:String(count)})),...body)}
function row(name,meta,state){return h("div",{class:"row"},h("div",{class:"name",text:name}),st(state),meta?h("div",{class:"meta"},meta):null)}
async function copy(text,btn){try{await navigator.clipboard.writeText(text);const was=btn.textContent;btn.textContent="Copied";setTimeout(()=>btn.textContent=was,1200);say("Copied to clipboard.")}catch{say("Clipboard unavailable; select the text to copy it.")}}
const go=(r,f)=>show($("tab-"+r),f);
function home(c){const el=$("home");el.textContent="";if(!c){el.append(h("div",{class:"empty",text:"Project state is unavailable; no health claim is made. Reload after resolving the error."}));return}
if(!c.established)return onboarding(c,el);
const inForce=c.rules.filter(r=>r.lifecycle==="accepted");
el.append(h("div",{class:"viewhead"},h("h1",{id:"mission-line",text:c.mission?.title||"Mission not stated"}),h("p",{class:"sub",text:c.principles.length+(c.principles.length===1?" principle, ":" principles, ")+inForce.length+(inForce.length===1?" rule in force":" rules in force")})));
const [p,...rest]=c.attention;
el.append(panel("Needs attention",c.attention.length||"none",p?h("div",{class:"attn",id:"attention-primary"},h("div",{class:"head"},h("h3",{text:p.title}),st({tone:p.tone,label:p.kind_label})),h("p",{text:p.text}),h("dl",{class:"next"},h("dt",{text:"For"}),h("dd",{text:p.actor}),h("dt",{text:"Next"}),h("dd",{text:p.next})),h("div",{class:"act"},h("button",{class:"btn primary",type:"button",text:p.action_label,onclick:()=>go(p.route,p.focus)}),p.agent_instruction?h("button",{class:"btn quiet",type:"button",text:"Copy agent brief",onclick:e=>copy(p.agent_instruction,e.currentTarget)}):null)):h("div",{class:"attn"},h("div",{class:"head"},h("h3",{text:"Nothing needs you right now."}),st({tone:"muted",label:"up to date"})),h("p",{text:"Every rule has a current result and no hand is raised. Run checks after changes."})),rest.map(i=>h("div",{class:"attn more"},h("div",{class:"t",text:i.title}),h("div",{class:"m",text:i.actor+" · "+i.next}),h("button",{class:"btn quiet go",type:"button",text:i.action_label,onclick:()=>go(i.route,i.focus)})))));
el.append(panel("Rules",inForce.length,h("div",{class:"rows",id:"home-rules"},inForce.length?inForce.map(r=>row(r.title,[r.strength+" · "+r.enforcer+" · "+r.runs_at,r.shadow?h("span",{class:"badge",text:"shadow"}):null],r.result)):h("div",{class:"empty",text:"No rule is in force, so nothing is checked yet."}))));
if(c.onboarding.steps.some(s=>!s.done&&!s.optional))el.append(panel("Setup",null,steps(c),h("div",{class:"act"},h("code",{class:"cmd",text:"wh init --action wire --hooks"}))));
if(c.latest_change)el.append(h("button",{class:"footrow",type:"button",onclick:()=>go("changelog")},h("span",{class:"k",text:"Latest change"}),h("span",{class:"v"},c.latest_change.title,h("time",{text:date(c.latest_change.at)+" "+time(c.latest_change.at)}))))}
const api={h,st,$,W,date,time,say,cmd,copy,panel,row,csrf,reload:q=>load(q),show:go};
async function views(){if(!W.views){const l=h("link",{rel:"stylesheet",href:"/views.css"});d.head.append(l);await new Promise(r=>{l.onload=r;l.onerror=r});W.views=await import("/views.js")}return W.views}
async function editor(){await views();return W.edit||=await import("/edit.js")}
api.editor=editor;
function render(){const c=W.value?.data?.current;header(c);home(c);if(W.views)W.views.render(api)}
async function load(q){try{const r=await inspect(q);W.value=r;W.journal=r.data?.changelog||[];render();return r}catch(e){W.value=null;render();say(String(e.message||e))}}
async function show(tab,focus){if(!tab)return;if(tab.getAttribute("aria-disabled")==="true"){say("Available once the agreement has a mission and a rule.");return}for(const t of tabs()){const on=t===tab;t.setAttribute("aria-selected",String(on));t.tabIndex=on?0:-1;$(t.getAttribute("aria-controls")).hidden=!on}tab.focus();say(tab.firstChild.textContent+" view");if(tab.id!=="tab-home"){await views();W.views.render(api)}if(focus){const panel=$(tab.getAttribute("aria-controls")),n=panel.querySelector('[data-id="'+CSS.escape(focus)+'"]');if(n){n.setAttribute("tabindex","-1");n.scrollIntoView({block:"start"});n.focus({preventScroll:true})}}}
d.addEventListener("DOMContentLoaded",async()=>{theme(saved);for(const b of d.querySelectorAll(".theme button"))b.addEventListener("click",()=>theme(b.dataset.theme));
for(const t of tabs()){t.addEventListener("click",()=>show(t));t.addEventListener("keydown",e=>{const k={ArrowRight:1,ArrowLeft:-1}[e.key];const all=tabs();let i=all.indexOf(t);if(k)i=(i+k+all.length)%all.length;else if(e.key==="Home")i=0;else if(e.key==="End")i=all.length-1;else return;e.preventDefault();all[i].focus();show(all[i])})}
$("where").textContent="wh dash · "+location.host+" · local, private, unpublished";
const b=new URLSearchParams(location.hash.slice(1)).get("bootstrap");history.replaceState(null,"",location.pathname);
if(b){try{const r=await json(await fetch("/session/bootstrap",{method:"POST",headers:{"X-Whetstone-Bootstrap":b}}));sessionStorage.setItem("whetstone_csrf",r.csrf_token)}catch(e){say("Edit capability was not granted: "+e.message)}}
await load()})})();
