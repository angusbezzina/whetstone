let h,st,dt;const chev='M6 3l5 5-5 5',D=(c,...x)=>h("div",{class:c},...x),B=(c,o,...x)=>h("button",{class:"btn "+c,type:"button",...o},...x),I=()=>icon(chev),T=(t,c,x)=>h(t,{class:c||null,text:x});
function icon(path){const n="http://www.w3.org/2000/svg",s=document.createElementNS(n,"svg"),p=document.createElementNS(n,"path");s.setAttribute("viewBox","0 0 16 16");s.setAttribute("fill","none");s.setAttribute("stroke","currentColor");s.setAttribute("stroke-width","1.5");s.setAttribute("aria-hidden","true");p.setAttribute("d",path);s.append(p);return s}
const F={q:"",asOf:""};
const PREFIX={principle:"principle",rule:"rule"};
const GROUPS=[["must","Must","fails the check until repaired"],["should","Should","flags for review; never blocks"]];
const ed=(a,fn,...x)=>e=>{const t=e.currentTarget;a.editor().then(m=>m[fn](a,...x,t))};
function head(title,sub,...extra){return D("viewhead",T("h1",0,title),T("p","sub",sub),...extra)}
function acts(a,x,host){return a.csrf()?D("e",B("quiet",{text:"Edit","aria-expanded":"false",onclick:ed(a,"open",x.edit,host)}),x.kind!=="mission"&&x.lifecycle!=="new"?B("quiet danger",{text:"Retire","aria-expanded":"false",onclick:ed(a,"retire",x,host)}):null):null}
function detail(x){return x.detail.length?D("d",x.detail.map((q,i)=>[i?" · ":null,T("span","k",q.key+" "),q.code?T("code",0,q.value):q.value])):null}
function rec(a,x,big,body){const r=h("div",{class:"rec"+(x.lifecycle!=="accepted"?" draft":""),"data-id":x.id});r.append(...[h("div",{class:"c"+(big?" big":""),text:x.title}),h("span",{class:"st state "+x.state.tone,text:x.state.label}),x.digest?D("ver",T("b",0,x.digest.slice(7,19))):null,acts(a,x,r),body||detail(x),D("editor",h("div"))].filter(Boolean));return r}
function adder(a,base,label,host){return a.csrf()?D("addrow",B("quiet",{text:"+ "+label,onclick:ed(a,"open",base,host)})):null}
function panel(a,id,title,why,items,draw,add,empty){const p=h("section",{class:"panel",id},D("ph",h("h2",{},title,T("span","why",why)),T("span","count",String(items.length))));p.append(...items.map(draw));if(!items.length)p.append(T("div","empty",empty||"Nothing in the rulebook yet."));const b=add&&adder(a,add[0],add[1],p);if(b)p.append(b,D("editor",h("div")));return p}
function examples(r){return r.examples.length?h("details",{class:"ex"},h("summary",{},I(),r.examples.length+" labelled example"+(r.examples.length===1?"":"s")),h("ul",{},r.examples.map(x=>h("li",{},st({tone:x.expected==="flag"?"fail":"pass",label:x.expected}),T("span",0,x.reason),T("pre",0,x.input))))):null}
const K=(k,...v)=>[T("span","k",k+" "),...v];
function rule(a,r){return rec(a,r,false,D("d",D("enf",r.enforcer+" ",r.command?T("code",0,r.command):null),h("div",{},K("runs at",r.runs_at),r.paths.length?[" · ",K("paths",r.paths.join(", "))]:null),h("div",{},K("sweep",r.sweep_class.replace("_"," "))," · ",K("priority",String(r.sweep_priority)),r.sweep_unsure.length?[" · ",K("unsure",r.sweep_unsure.map(u=>u.reason).join("; "))]:null),r.rationale?h("div",{},K("why",r.rationale)):null,r.source?h("div",{},K("source",r.source)):null,examples(r)))}
function drafts(a,c){const p=h("section",{class:"panel",id:"r-drafts","data-id":"drafts"},D("ph",h("h2",{},"Drafts",T("span","why","edits in whetstone/rulebook.json that are not committed")),T("span","count",String(c.drafts.length))),D("rows",c.drafts.map(x=>D("row",T("div","name",x)))));
if(a.csrf()){const host=D("editor",h("div"));p.append(D("act",B("primary",{text:"Accept",id:"drafts-accept",onclick:ed(a,"settle",c,"accept",host)}),B("",{text:"Withdraw",id:"drafts-withdraw",onclick:ed(a,"settle",c,"withdraw",host)})),host)}return p}
function rules(a){const el=a.$("rules"),c=a.W.value?.data?.current;el.textContent="";if(!c)return;
el.append(head("Rules","The mission, your principles and the rules drawn from them, as whetstone/rulebook.json holds them. Each rule has one strength and one mechanical enforcer. An edit is a draft in the file until you accept it, which commits it."));
if(c.drafts.length)el.append(drafts(a,c));
if(!c.established&&!c.rules.length&&!c.mission){el.append(T("p","notice","The rulebook is empty. Start onboarding on Home."),B("",{text:"Open onboarding",onclick:()=>a.show("home","onboard")}));return}
const by=s=>c.rules.filter(r=>r.strength===s);
el.append(panel(a,"r-mission","Mission","why the project exists",c.mission?[c.mission]:[],x=>rec(a,x,true),c.mission?null:[{kind:"mission",record_id:null,content:"",definition:null},"State the mission"]));
el.append(panel(a,"r-principles","Principles","what the rules are drawn from",c.principles,x=>rec(a,x),[{kind:"principle",record_id:null,content:"",definition:null},"Add principle"]));
for(const[s,t,why]of GROUPS)el.append(panel(a,"r-"+s,t,why,by(s),x=>rule(a,x),[{kind:"rule",record_id:null,content:"",definition:{type:"rule",strength:s,enforcer:{kind:"test",command:""}}},"Add "+s+" rule"]));
if(a.csrf()){const ta=h("textarea",{id:"compile-text","aria-label":"Rules in prose",placeholder:"Paste CONTRIBUTING, a style guide or review comments: one rule per line or bullet."}),out=h("div",{class:"compiled","aria-live":"polite"});el.append(h("section",{class:"panel",id:"r-compile"},D("ph",h("h2",{},"Draft rules from text",T("span","why","each rule that names its check is probed and drafted; the rest need an enforcer"))),ta,D("act",B("primary",{id:"compile-btn",text:"Draft rules",onclick:e=>{const t=e.currentTarget;if(ta.value.trim())a.editor().then(m=>m.compile(a,ta.value,t,out))}})),out))}}
function changelog(a){const el=a.$("changelog"),c=a.W.value?.data?.current,o=a.$("cl-q"),sel=o&&document.activeElement===o?[o.selectionStart,o.selectionEnd]:null;el.textContent="";if(!c?.established)return;
el.append(head("Changelog","The commits that changed whetstone/rulebook.json, newest first: what changed, who committed it and why."));
const q=h("input",{type:"search",id:"cl-q",value:F.q,placeholder:"subject, message, author, commit"}),at=h("input",{type:"datetime-local",id:"cl-asof",value:F.asOf});
let timer;q.addEventListener("input",()=>{F.q=q.value;F.n=0;clearTimeout(timer);timer=setTimeout(()=>a.reload(query()),250)});
at.addEventListener("change",()=>{F.asOf=at.value;a.reload(query()).then(()=>a.$("cl-asof")?.focus())});
el.append(D("toolbar",h("label",{},"Search",q),h("label",{},"As of",at),F.asOf?T("span","asof-note","showing the log as of "+dt(F.asOf)):null,h("a",{class:"export",href:"/api/trail.tsv",download:"whetstone-decisions.tsv",title:"One row per commit: ts, phase, decision, why, evidence, result",text:"Export decision trail (TSV)"})));if(sel){q.focus();q.setSelectionRange(...sel)}
const items=a.W.journal,shown=items.slice(0,F.n||100);const days=new Map;for(const j of shown){const k=a.date(j.recorded_at);days.has(k)||days.set(k,[]);days.get(k).push(j)}
if(!items.length)el.append(T("div","empty",F.q?"Nothing matches that search. Clear it or widen the as-of time.":"No commit has touched the rulebook yet. Accept the drafts in Rules to make the first."));
for(const[day,list]of days)el.append(h("section",{class:"day","aria-label":day},h("h2",{},day,T("span",0,list.length+(list.length===1?" commit":" commits"))),list.map(j=>entry(a,j))));
if(items.length>shown.length)el.append(h("button",{class:"btn",id:"cl-more",type:"button",text:"Load more ("+(items.length-shown.length)+" older)",onclick:()=>{F.n=shown.length+100;changelog(a)}}));
if(c.retired.length)el.append(a.panel("Retired",c.retired.length,h("div",{class:"rows",id:"cl-retired"},c.retired.map(r=>a.row(r.id,r.reason,{tone:"muted",label:"retired"})))))}
function coverage(a){const el=a.$("coverage"),c=a.W.value?.data?.current,v=c?.coverage;el.textContent="";if(!c?.established||!v)return;
el.append(head("Coverage","Where the rulebook is thin: parts of the repository no rule reads, and rules that read nothing. A rule reads a file when its paths cover it and its enforcer reads that kind of file."));
el.append(D("checkline",h("span",{},T("b",0,v.covered+" of "+v.files)," files are read by at least one rule")));
el.append(panel(a,"cov-uncovered","Not read by any rule","by top-level directory and language",v.uncovered,u=>a.row(u.directory+" · "+u.language,u.files+" file"+(u.files===1?"":"s")+" · "+u.sample.join(", "),{tone:"warn",label:"uncovered"}),null,"Every file is read by at least one rule."));
el.append(panel(a,"cov-dead","Rules that read nothing","their paths or enforcer match no file here",v.dead_rules,d=>a.row(d.statement,d.rule+" · "+d.enforcer+" · "+d.why,{tone:"muted",label:"reads nothing"}),null,"Every rule reads at least one file."))}
function query(){return{search:F.q.trim()||null,as_of:F.asOf?new Date(F.asOf).toISOString().replace(/\.\d+Z$/,"Z"):null,page_size:100}}
function entry(a,j){return h("article",{class:"entry","data-id":j.id},h("time",{datetime:j.recorded_at,text:a.time(j.recorded_at)}),D("t",j.title,T("span","ver",j.short)),h("span",{class:"state status pass",text:"committed"}),j.summary?T("div","s",j.summary):null,T("div","m",j.owner))}
export function render(a){h=a.h;st=a.st;dt=v=>a.date(v)+" "+a.time(v);rules(a);changelog(a);coverage(a)}
export const prefix=PREFIX;
