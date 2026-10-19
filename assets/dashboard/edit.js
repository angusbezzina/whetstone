let h;const slug=t=>t.toLowerCase().replace(/[^a-z0-9]+/g,"-").replace(/^-|-$/g,"").slice(0,40)||"item";
const lines=v=>(v||"").split("\n").map(x=>x.trim()).filter(Boolean);
const ids=v=>(v||"").split(/[\s,]+/).map(x=>x.trim()).filter(Boolean);
const err=e=>String(e.message||e);
const FILE="whetstone/rulebook.json";
const LAB={test:"Test command",lint:"Lint tool",formatter:"Formatter",ast:"AST query (a match is a violation)",validator:"Validator command"};
const SWEEP=["discuss","confirm_first","draft_pr"];
function field(a,label,name,value,opts){const o=opts||{};let input;if(o.choices){input=h("select",{name},o.choices.map(c=>h("option",{value:c,text:c.replace(/_/g," ")})));input.value=value||o.choices[0]}else if(o.area){input=h("textarea",{name,placeholder:o.hint||""});input.value=value||""}else input=h("input",{name,value:value||"",placeholder:o.hint||""});if(o.required)input.required=true;return h("label",{},label,input)}
function val(en){const e=en||{};return e.query??e.command??e.tool??""}
function enforcer(k,v,old){const x=(v.value||"").trim(),e=old?.kind===k?{...old}:{kind:k};
if(k==="ast")e.query=x;else if(k==="lint"){e.tool=x;e.code=v.code.trim()}else if(k==="formatter")e.tool=x;else if(k==="test"||k==="validator")e.command=x;return e}
// One doubt per line: where (globs, or /pattern/ for the flagged text), a colon, then the reason.
function unsureText(u){return(u||[]).map(x=>[...(x.paths||[]),...(x.matching!=null?["/"+x.matching+"/"]:[])].join(", ")+": "+x.reason).join("\n")}
function unsure(v){return lines(v).map(l=>{const i=l.indexOf(": "),o={reason:(i<0?l:l.slice(i+2)).trim()},p=[];for(const x of(i<0?"":l.slice(0,i)).split(",").map(s=>s.trim()).filter(Boolean))if(x.length>1&&x[0]==="/"&&x.endsWith("/"))o.matching=x.slice(1,-1);else p.push(x);if(p.length)o.paths=p;return o})}
function fields(a,base){const k=base.kind,d=base.definition||{},en=d.enforcer||{};const f=[field(a,{mission:"Mission, in one sentence",principle:"Principle",rule:"Rule, as one plain sentence"}[k],"content",base.content,{area:true,required:true})];
if(k==="rule"){const vf=field(a,LAB[en.kind]||"Command","value",val(en),{required:true}),kind=field(a,"Enforcer","enforcer",en.kind,{choices:Object.keys(LAB)});
const sync=()=>{const s=kind.lastChild.value;vf.firstChild.textContent=LAB[s]};kind.lastChild.addEventListener("change",sync);
f.push(h("div",{class:"two"},field(a,"Strength","strength",d.strength,{choices:["must","should"]}),kind),vf,h("div",{class:"two"},field(a,"Lint code (lint only)","code",en.code),field(a,"Paths (globs, comma separated)","paths",(d.paths||[]).join(", "))));queueMicrotask(sync);
f.push(h("div",{class:"two"},field(a,"Sweep class","sweep",d.sweep||"discuss",{choices:SWEEP}),field(a,"Sweep priority (0 is most urgent)","sweep_priority",String(d.sweep_priority??2),{choices:["0","1","2","3","4"]})),field(a,"Unsure, one per line: paths or /pattern/, a colon, the reason","sweep_unsure",unsureText(d.sweep_unsure),{area:true,hint:"tests/**: fixtures print on purpose"}))}
if(k!=="mission")f.push(field(a,k==="rule"?"Why this rule exists":"Why (optional)","rationale",base.rationale,{area:true,required:k==="rule",hint:"a reviewer reads this"}));return f}
function definition(base,v){const k=base.kind,d=base.definition||{};
if(k==="rule"){const p=+v.sweep_priority;return{...d,type:"rule",strength:v.strength,enforcer:enforcer(v.enforcer,v,d.enforcer),paths:ids(v.paths),sweep:v.sweep==="discuss"&&d.sweep==null?null:v.sweep,sweep_priority:p===2&&d.sweep_priority==null?null:p,sweep_unsure:unsure(v.sweep_unsure)}}
if(k==="principle")return{type:"principle",pstack:d.pstack??null,rationale:(v.rationale||"").trim()||null};return null}
function close(host,trigger){host.classList.remove("open");host.firstElementChild.textContent="";trigger?.setAttribute("aria-expanded","false")}
function closeAll(){for(const o of document.querySelectorAll(".editor.open"))close(o)}
function effects(){return h("div",{class:"effects"},[["draft in "+FILE,"written",true],["committed","no"],["enforced by CI","no"]].map(([k,v,y])=>h("span",{class:y?"y":"n"},k+" ",h("b",{text:v}))))}
function show(v){return typeof v==="string"?v:v==null?"—":JSON.stringify(v,null,1)}
function busy(b,on){b?.classList.toggle("busy",on);if(b)b.disabled=on}
const fail=(box,r)=>box.prepend(h("p",{class:"notice fail",text:r.blocking_questions?.[0]||r.summary}));
function cancel(a,host,trigger){return h("button",{class:"btn",type:"button",text:"Cancel",onclick:()=>{close(host,trigger);trigger?.focus();a.say("Cancelled; the rulebook was not changed.")}})}
// Open a form in the editor under `row`, preview its request, then confirm it.
function flow(a,row,trigger,intro,body,submit,request,review,confirm){const host=row.classList.contains("editor")?row:row.querySelector(":scope > .editor"),box=host.firstElementChild;closeAll();
const form=h("form",{},h("div",{class:"base"},...intro),body,h("div",{class:"act"},cancel(a,host,trigger),h("button",{class:"btn primary",type:"submit",text:submit})));
box.textContent="";box.append(form);host.classList.add("open");trigger?.setAttribute("aria-expanded","true");form.querySelector("input,textarea,select")?.focus();
form.addEventListener("submit",async ev=>{ev.preventDefault();const v=Object.fromEntries(new FormData(form)),req=request(v);try{const pre=await a.cmd({...req,preview:true});if(!pre.data?.preview_only)return fail(box,pre);
box.textContent="";box.append(h("div",{class:"review",role:"group","aria-label":submit},review(pre,v),h("div",{class:"confirm"},h("button",{class:"btn",type:"button",text:"Back",onclick:()=>flow(a,row,trigger,intro,body,submit,request,review,confirm)}),h("button",{class:"btn primary",type:"button",text:confirm,id:"confirm-edit",onclick:async e=>{const b=e.currentTarget;busy(b,true);try{const done=await a.cmd(req);await a.reload();a.say(done.summary)}catch(x){busy(b,false);fail(box,{summary:err(x)})}}}))));box.querySelector("#confirm-edit").focus()}catch(x){box.prepend(h("p",{class:"notice fail",text:err(x)}))}})}
function diff(before,after,label){return h("div",{class:"diff"},h("div",{class:"before"},h("div",{class:"lab",text:"before"}),h("pre",{text:show(before)})),h("div",{class:"after"},h("div",{class:"lab",text:label}),h("pre",{text:show(after)})))}
export function open(a,base,row,trigger){h=a.h;const id0=base.record_id;
flow(a,row,trigger,[h("span",{},id0?"editing ":"new ",h("b",{text:id0||base.kind})),h("span",{text:"written to "+FILE+" as a draft; accepting commits it"})],fields(a,base),"Review edit",
v=>({workflow:"change",kind:base.kind,record_id:base.kind==="mission"?null:id0||a.W.views.prefix[base.kind]+"."+slug(v.content),content:v.content.trim(),rationale:base.kind==="mission"?null:(v.rationale||"").trim()||null,definition:definition(base,v)}),
(pre,v)=>[h("div",{class:"base"},h("span",{},"the exact entry in ",h("b",{text:FILE})),pre.data.digest?.after?h("span",{text:"digest "+pre.data.digest.after.slice(0,19)+"…"}):null),diff(pre.data.diff.before,pre.data.diff.after,"after · draft"),v.rationale?h("div",{class:"why"},h("span",{class:"k",text:"why "}),v.rationale):null,effects()],"Write draft")}
export function retire(a,x,row,trigger){h=a.h;
flow(a,row,trigger,[h("span",{},"retiring ",h("b",{text:x.id})),h("span",{text:"it leaves the rulebook; the reason stays in its retired list and in the commit"})],field(a,"Why is it retired?","rationale","",{area:true,required:true}),"Review retirement",
v=>({workflow:"change",retire:x.id,rationale:v.rationale.trim()}),
pre=>[diff(pre.data.diff.before,{retired:{id:x.id,reason:pre.data.reason}},"after · draft"),effects()],"Write retirement draft")}
export function settle(a,c,verdict,host,trigger){h=a.h;const acc=verdict==="accept";
flow(a,host,trigger,[h("span",{},acc?"accepting ":"withdrawing ",h("b",{text:c.drafts.length+(c.drafts.length===1?" edit":" edits")})),h("span",{text:acc?"commits "+FILE+" and nothing else":"restores "+FILE+" to its last commit"})],acc?field(a,"Why (in the commit message, optional)","rationale","",{area:true}):h("p",{text:"Every draft edit below is discarded."}),acc?"Review commit":"Review withdrawal",
v=>({workflow:"change",[verdict]:true,rationale:acc?(v.rationale||"").trim()||null:null}),
pre=>[acc?h("div",{},h("p",{},h("b",{text:pre.data.message.subject})),pre.data.message.body?h("pre",{class:"brief",text:pre.data.message.body}):null):null,acc?null:h("div",{class:"rows"},pre.data.discards.map(x=>a.row(x,null,{tone:"muted",label:"discard"})))],acc?"Commit rulebook":"Withdraw edits")}
const ck=(a,name,value,on,label,side,note,extra)=>h("label",{class:"pick"},h("input",{type:"checkbox",name,value,checked:on,disabled:extra?.disabled}),h("b",{text:label}),h("span",{class:"g",text:side}),h("span",{class:"s",text:note}),extra?.more);
export function onboard(a,el){h=a.h;const c=a.W.value.data.current,o=c.onboarding,held=new Set(c.principles.map(p=>p.id));
const form=h("form",{id:"init-form"},h("div",{class:"ph"},h("h2",{text:"Agree the rules"}),h("span",{class:"count",text:"drafts in "+FILE})),
h("fieldset",{},h("legend",{},"Mission",h("small",{text:"what this project exists to do"})),field(a,"Mission, in one sentence","mission",c.mission?.title,{area:true,required:true})),
h("fieldset",{id:"ob-principles"},h("legend",{},"Principles",h("small",{text:"catalogue "+o.catalogue_version+"; pick the ones you hold"})),o.catalogue.map(p=>ck(a,"principle",p.id,held.has("principle."+p.id),p.id.replace(/-/g," "),p.enforceable,p.rule,{disabled:held.has("principle."+p.id)})),field(a,"Your own principles, one per line","custom_principles","",{area:true})),
h("fieldset",{id:"ob-starters"},h("legend",{},"Starter rules",h("small",{text:"each is checked the cheapest reliable way"})),o.starters.map(s=>ck(a,"starter",s.id,true,s.title,s.strength+" · "+s.enforcer+(s.accepted?" · in the rulebook":""),s.detected,{disabled:s.accepted,more:s.examples.length?h("details",{},h("summary",{text:s.examples.length+" labelled examples"}),s.examples.map(x=>h("pre",{text:x.expected+": "+x.reason+"\n"+x.input}))):null}))),
h("fieldset",{},h("legend",{},"Gates",h("small",{text:"after the rulebook"})),h("p",{class:"sub",text:"Install the pre-commit and pre-push hooks with:"}),h("code",{class:"cmd",text:"wh init --action wire --hooks"})),
h("div",{class:"confirm"},h("span",{class:"note",text:"Review lists each edit first. They are drafts in "+FILE+" until you accept them, which commits the file."}),h("button",{class:"btn primary",type:"submit",text:"Review drafts"})));
el.textContent="";el.append(form);form.querySelector("textarea")?.focus();
form.addEventListener("submit",ev=>{ev.preventDefault();const fd=new FormData(form),m=fd.get("mission").trim(),edits=[];
if(m!==(c.mission?.title||""))edits.push({kind:"mission",content:m});
for(const id of fd.getAll("principle")){const p=o.catalogue.find(x=>x.id===id);edits.push({kind:"principle",record_id:"principle."+id,content:p.rule,definition:{type:"principle",pstack:id}})}
for(const t of lines(fd.get("custom_principles")))edits.push({kind:"principle",record_id:"principle.custom-"+slug(t),content:t});
for(const id of fd.getAll("starter")){const s=o.starters.find(x=>x.id===id).edit;edits.push({kind:"rule",record_id:s.record_id,content:s.content,rationale:s.rationale,definition:s.definition})}
if(!edits.length){form.prepend(h("p",{class:"notice fail",text:"Nothing new to draft: change the mission, pick a principle or a starter rule."}));return}
el.textContent="";el.append(h("div",{class:"review"},h("h2",{text:"Review the drafts"}),h("p",{text:"These edits are written to "+FILE+" in your working tree. Nothing is committed or shared until you accept them in Rules."}),h("div",{class:"rows"},edits.map(e=>a.row(e.kind+": "+e.content,e.record_id,{tone:"draft",label:"draft"}))),h("div",{class:"confirm"},h("button",{class:"btn",type:"button",text:"Back",onclick:()=>onboard(a,el)}),h("button",{class:"btn primary",type:"button",id:"confirm-init",text:"Write drafts",onclick:async e=>{const b=e.currentTarget,stop=t=>{busy(b,false);el.firstElementChild.prepend(h("p",{class:"notice fail",text:t}))};busy(b,true);try{for(const x of edits){const r=await a.cmd({workflow:"change",...x});if(r.state!=="success")return stop((x.record_id||"mission")+": "+(r.blocking_questions?.[0]||r.summary)+" The edits before it are drafts in "+FILE+"; writing again skips them.")}await a.reload();a.say("The drafts are in "+FILE+". Accept them in Rules.");a.show("rules","drafts")}catch(x){stop(err(x))}}}))));document.getElementById("confirm-init")?.focus()})}
