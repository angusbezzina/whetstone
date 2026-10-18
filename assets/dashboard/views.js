let h,st,dt;const chev='M6 3l5 5-5 5',D=(c,...x)=>h("div",{class:c},...x),B=(c,o,...x)=>h("button",{class:"btn "+c,type:"button",...o},...x),I=()=>icon(chev),T=(t,c,x)=>h(t,{class:c||null,text:x});
function icon(path){const n="http://www.w3.org/2000/svg",s=document.createElementNS(n,"svg"),p=document.createElementNS(n,"path");s.setAttribute("viewBox","0 0 16 16");s.setAttribute("fill","none");s.setAttribute("stroke","currentColor");s.setAttribute("stroke-width","1.5");s.setAttribute("aria-hidden","true");p.setAttribute("d",path);s.append(p);return s}
const F={q:"",asOf:"",kind:"decision"};
const PREFIX={mission:"mission",principle:"principle",rule:"rule"};
const GROUPS=[["must","Must","fails the check until repaired"],["should","Should","flags for review; never blocks"]];
const ed=(a,fn,...x)=>e=>{const t=e.currentTarget;a.editor().then(m=>m[fn](a,...x,t))};
function head(title,sub,...extra){return D("viewhead",T("h1",0,title),T("p","sub",sub),...extra)}
function ver(x){return D("ver",T("b",0,"v"+x.version)," · ",x.lifecycle==="draft"?T("i",0,"draft"):"accepted",x.pending?[h("br"),T("i",0,"draft v"+x.pending.version+" pending")]:null)}
function edit(a,x,host){return a.csrf()?D("e",B("quiet",{text:"Edit","aria-expanded":"false",onclick:ed(a,"open",x.edit,host)})):null}
function detail(x){const d=x.detail.filter(q=>q.key!=="summary");return d.length?D("d",d.map((q,i)=>[i?" · ":null,T("span","k",q.key+" "),q.code?T("code",0,q.value):q.value])):null}
function rec(a,x,big,body){const r=h("div",{class:"rec"+(x.lifecycle==="draft"||x.pending?" draft":""),"data-id":x.id});r.append(...[h("div",{class:"c"+(big?" big":""),text:x.title}),x.state&&h("span",{class:"st state "+x.state.tone,text:x.state.label}),ver(x),edit(a,x,r),body||detail(x),D("editor",h("div"))].filter(Boolean));return r}
function adder(a,base,label,host){return a.csrf()?D("addrow",B("quiet",{text:"+ "+label,onclick:ed(a,"open",base,host)})):null}
function panel(a,id,title,why,items,draw,add){const p=h("section",{class:"panel",id},D("ph",h("h2",{},title,T("span","why",why)),T("span","count",String(items.length))));p.append(...items.map(draw));if(!items.length)p.append(T("div","empty","Nothing recorded yet."));const b=add&&adder(a,add[0],add[1],p);if(b)p.append(b,D("editor",h("div")));return p}
function examples(r){return r.examples.length?h("details",{class:"ex"},h("summary",{},I(),r.examples.length+" labelled example"+(r.examples.length===1?"":"s")),h("ul",{},r.examples.map(x=>h("li",{},st({tone:x.expected==="flag"?"fail":"pass",label:x.expected}),T("span",0,x.reason),T("pre",0,x.input))))):null}
function rule(a,r){return rec(a,r,false,D("d",D("enf",r.enforcer+" ",r.command?T("code",0,r.command):null),h("div",{},T("span","k","runs at "),r.runs_at,r.paths.length?[" · ",T("span","k","paths "),r.paths.join(", ")]:null," · ",T("span","k","sweep "),r.sweep_class.replace("_"," ")),r.source?h("div",{},T("span","k","source "),r.source):null,examples(r)))}
function rules(a){const el=a.$("rules"),c=a.W.value?.data?.current;el.textContent="";if(!c)return;
el.append(head("Rules","The mission, your principles and the rules drawn from them. Each rule has one strength and one mechanical enforcer. Edits are private drafts until you accept them."));
if(!c.established&&!c.rules.length){el.append(T("p","notice","No rule is agreed yet. Start onboarding on Home, or run wh init."),B("",{text:"Open onboarding",onclick:()=>a.show("home","onboard")}));return}
const by=s=>c.rules.filter(r=>r.strength===s);
el.append(panel(a,"r-mission","Mission","why the project exists",c.mission?[c.mission]:[],x=>rec(a,x,true),c.mission?null:[{kind:"mission",record_id:"mission.project",content:"",definition:null},"State the mission"]));
el.append(panel(a,"r-principles","Principles","what the rules are drawn from",c.principles,x=>rec(a,x),[{kind:"principle",record_id:null,content:"",definition:null},"Add principle"]));
for(const[s,t,why]of GROUPS)el.append(panel(a,"r-"+s,t,why,by(s),x=>rule(a,x),[{kind:"rule",record_id:null,content:"",definition:{type:"rule",strength:s,enforcer:{kind:"test",command:""}}},"Add "+s+" rule"]));
const act=(p,id,text,fn)=>{if(a.csrf())p.append(D("act",B("",{id,text,onclick:ed(a,fn)})));el.append(p)};
const lg=c.onboarding.legacy.filter(x=>!x.migrated);if(lg.length)act(panel(a,"r-legacy","Earlier records","values, philosophy and YAML rules from before",lg,x=>a.row(x.text,x.kind,{tone:"muted",label:"not migrated"})),"migrate-btn","Draft principles and rules from these","migrate")}
function changelog(a){const el=a.$("changelog"),c=a.W.value?.data?.current,o=a.$("cl-q"),sel=o&&document.activeElement===o?[o.selectionStart,o.selectionEnd]:null;el.textContent="";if(!c?.established)return;
el.append(head("Changelog","The decision trail, newest first: how the mission, principles and rules changed and why."));
const q=h("input",{type:"search",id:"cl-q",value:F.q,placeholder:"title, summary, area"}),at=h("input",{type:"datetime-local",id:"cl-asof",value:F.asOf});
let timer;q.addEventListener("input",()=>{F.q=q.value;F.n=0;clearTimeout(timer);timer=setTimeout(()=>a.reload(query()),250)});
at.addEventListener("change",()=>{F.asOf=at.value;a.reload(query()).then(()=>a.$("cl-asof")?.focus())});
const kinds=h("span",{class:"filter",role:"group","aria-label":"Show"},[["decision","Decisions"],["all","All"]].map(([v,l])=>h("button",{type:"button","aria-pressed":String(F.kind===v),text:l,onclick:()=>{F.kind=v;F.n=0;changelog(a)}})));
el.append(D("toolbar",h("label",{},"Search",q),h("label",{},"As of",at),kinds,F.asOf?T("span","asof-note","showing the trail as of "+dt(F.asOf)):null,h("a",{class:"export",href:"/api/trail.tsv",download:"whetstone-decisions.tsv",title:"One row per decision: ts, phase, decision, why, evidence, result",text:"Export decision trail (TSV)"})));if(sel){q.focus();q.setSelectionRange(...sel)}
const items=a.W.journal.filter(j=>F.kind==="all"||j.kind===F.kind),shown=items.slice(0,F.n||100);const days=new Map;for(const j of shown){const k=a.date(j.recorded_at);days.has(k)||days.set(k,[]);days.get(k).push(j)}
if(!items.length)el.append(T("div","empty",F.q?"Nothing matches that search. Clear it or widen the as-of time.":"No entries of this kind yet."));
for(const[day,list]of days)el.append(h("section",{class:"day","aria-label":day},h("h2",{},day,T("span",0,list.length+(list.length===1?" change":" changes"))),list.map(j=>entry(a,j))));
if(items.length>shown.length)el.append(h("button",{class:"btn",id:"cl-more",type:"button",text:"Load more ("+(items.length-shown.length)+" older)",onclick:()=>{F.n=shown.length+100;changelog(a)}}))}
function query(){return{search:F.q.trim()||null,as_of:F.asOf?new Date(F.asOf).toISOString().replace(/\.\d+Z$/,"Z"):null,page_size:100}}
function entry(a,j){const e=h("article",{class:"entry","data-id":j.id},h("time",{datetime:j.recorded_at,text:a.time(j.recorded_at)}),D("t",j.title,j.version?T("span","ver",j.version):null),h("span",{class:"state status "+j.status.tone+" "+j.status.label,text:j.status.label}),T("div","s",j.summary),T("div","m",[j.owner,j.area,j.note].filter(Boolean).join(" · ")));
if(j.proposal&&a.csrf()){const host=D("editor",h("div"));e.append(D("act",B("primary",{text:"Accept",onclick:ed(a,"review",j,"accept",host)}),B("",{text:"Withdraw",onclick:ed(a,"review",j,"withdraw",host)})),host)}
e.append(h("details",{},h("summary",{},I(),"Exact records"),h("pre",{text:JSON.stringify(j.records.map(r=>r.record??{redacted:r.redaction_reason}),null,2).slice(0,20000)})));return e}
export function render(a){h=a.h;st=a.st;dt=v=>a.date(v)+" "+a.time(v);rules(a);changelog(a)}
export const prefix=PREFIX;
