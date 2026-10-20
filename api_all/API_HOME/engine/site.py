"""Front side: one static site over every paper folder.

papers_root/_SITE/
  index.html          dashboard (the EVIDENCE_DASHBOARD v0.3 sections, as HTML) + every collection + every paper
  index.tsv           MASTER_INDEX v0.3: 43 columns, one row per paper, read by the v0.3 source map
  validation.tsv      V1-V12 results (failures are listed, never fixed)
  papers/<id>.html    the stacked page: station layers on top (in route order), the original document below

Nothing here calls a model and nothing in a paper folder is changed.
"""
from __future__ import annotations
import html,json,os,re
from datetime import datetime,timezone
from pathlib import Path
from .papers import all_papers,run_order
from .paths import external
from .report import latest_runs

# Column contract: MASTER_INDEX_FORMAT_v0.3 (same list as 03_EVIDENCE/backside_SCRIPTS/master_index_parser.py).
COLUMNS=["paper_id","source_sha256","clean_title","chapter","content_type","domain","reader_category","governing_question",
 "one_sentence_finding","paper_rating","rating_awarded_by","evidence_status","formal_status","evd_state","evd_support",
 "evd_counter","evd_balance","evd_families","evd_coverage","evd_gated","evd_stable","evd_weakest_claim","coherence",
 "physical_event","total_claims","claims_with_falsifiers","hidden_premises","lean_targets_queued","predictions_logged",
 "bridges_registered","bridges_blank_lost","truth_predicates","source_paragraphs","original_argument_steps",
 "total_argument_steps","supplementary_arguments","weak_links_identified","semantic_provider","semantic_model",
 "processed_date","tags","claim_ids","lean_receipts"]
INTEGRITY={"total_claims":"Total claims","claims_with_falsifiers":"Claims with falsifiers","hidden_premises":"Hidden premises found",
 "lean_targets_queued":"Lean targets queued","predictions_logged":"Predictions logged","bridges_registered":"Bridges registered",
 "bridges_blank_lost":"Bridges with blank LOST column"}

# ---------- reading ----------
def scalar(v:str):
 v=v.strip()
 if v[:1] in "\"'" and v[-1:]==v[:1]: return v[1:-1]
 if v.startswith("[") and v.endswith("]"): return [scalar(x) for x in v[1:-1].split(",") if x.strip()]
 low=v.lower()
 if low in ("true","false"): return low=="true"
 if low in ("null","~",""): return None
 try: return int(v)
 except ValueError: pass
 try: return float(v)
 except ValueError: return v

def frontmatter(text:str)->dict:
 """The first --- ... --- block that holds key: value lines: plain front matter, or the ```yaml block the v0.3 layers
 write (which sits after a --- horizontal rule, so the first pair of --- lines is not always the block)."""
 lines=text.replace("\r\n","\n").split("\n"); marks=[i for i,l in enumerate(lines) if l.strip()=="---"]
 def opens(i):  # a block starts the file, or opens a ```yaml fence; any other --- is a horizontal rule
  before=[l.strip() for l in lines[:i] if l.strip()]
  return not before or before[-1].lower() in ("```yaml","```yml")
 for a,b in zip(marks,marks[1:]):
  if not opens(a): continue
  out={}
  for line in lines[a+1:b]:
   k,sep,v=line.partition(":")
   if sep and not line.startswith((" ","\t","#")) and k.strip() and " " not in k.strip(): out[k.strip()]=scalar(v)
  if out: return out
 return {}

def table_under(text:str,heading:str)->list[list[str]]:
 m=re.search(r"^#+\s*"+re.escape(heading)+r"\s*$",text,re.M)
 if not m: return []
 rows=[]
 for line in text[m.end():].splitlines()[1:]:
  s=line.strip()
  if not s:
   if rows: break
   continue
  if s.startswith("<!--"): continue
  if not s.startswith("|"): break
  cells=[c.strip() for c in s.strip("|").split("|")]
  if not all(re.fullmatch(r":?-{2,}:?",c) for c in cells): rows.append(cells)
 return rows[1:]  # drop header

def number(v):
 if isinstance(v,(int,float)) and not isinstance(v,bool): return v
 m=re.search(r"-?\d+(?:\.\d+)?",str(v or ""))
 return (float(m.group()) if "." in m.group() else int(m.group())) if m else None

def layers(paper:Path)->list[tuple[str,Path,str]]:
 """(station label, run folder, text) for each station's latest run; text is the .md if the station wrote one, else its json."""
 out=[]
 for label,run in latest_runs(paper):
  md,js=run/f"{label}.md",run/f"{label}.json"
  if md.is_file(): out.append((label,run,md.read_text(encoding="utf-8",errors="replace")))
  elif js.is_file(): out.append((label,run,js.read_text(encoding="utf-8",errors="replace")))
 return out

def paper_record(paper:Path)->dict:
 meta=json.loads((paper/"paper.json").read_text(encoding="utf-8"))
 source=(paper/"00_SOURCE"/meta["source_file"]).read_text(encoding="utf-8",errors="replace")
 stack=layers(paper); fm=frontmatter(source); claims_text=""; claims_fm={}
 for _,_,text in stack:
  f=frontmatter(text)
  if f.get("type")=="claims_layer": claims_fm,claims_text=f,text
  elif f.get("type") in ("evidence-sheet","axiom_companion"): fm={**fm,**f}
 row={c:fm.get(c) for c in COLUMNS}
 row.update(paper_id=fm.get("paper_id") or meta.get("id"),source_sha256=meta.get("source_hash"),clean_title=fm.get("clean_title") or meta.get("title"))
 integrity={r[0]:r[1] for r in table_under(claims_text,"Layer integrity") if len(r)>1}
 for col,label in INTEGRITY.items(): row[col]=number(integrity.get(label)) if claims_text else None
 m=re.search(r"(\d+) predicates extracted from (\d+) source paragraphs",source)
 if m: row["truth_predicates"],row["source_paragraphs"]=int(m.group(1)),int(m.group(2))
 for r in table_under(source,"Argument score assessment"):
  if r and r[0]=="Originality" and len(r)>1:
   m=re.search(r"(\d+)\s*/\s*(\d+)"," ".join(r[1:]))
   if m: row["original_argument_steps"],row["total_argument_steps"]=int(m.group(1)),int(m.group(2))
 sup=len(re.findall(r"^\*\*Supplementary argument \d+\*\*",source,re.M)); row["supplementary_arguments"]=sup or row["supplementary_arguments"]
 weak=table_under(source,"Argument strengthening"); row["weak_links_identified"]=len(weak) if weak else row["weak_links_identified"]
 row["semantic_provider"]=row["semantic_provider"] or claims_fm.get("semantic_provider")
 row["semantic_model"]=row["semantic_model"] or claims_fm.get("semantic_model")
 claim_ids={r[0] for r in table_under(claims_text,"Claims table") if r}
 return {"paper":paper,"meta":meta,"row":row,"stack":stack,"source":source,"claims_fm":claims_fm,"claim_ids":claim_ids,"has_claims":bool(claims_text)}

# ---------- V1-V12 ----------
def validate(rec:dict)->list[tuple[str,str,str]]:
 r,out=rec["row"],[]
 def add(rule,level,msg): out.append((rule,level,msg))
 rating,by=number(r["paper_rating"]),str(r["rating_awarded_by"] or "")
 if rec["has_claims"] and rec["claims_fm"].get("source_sha256") not in (None,r["source_sha256"]): add("V1","HARD FAIL","claims layer source_sha256 does not match the paper")
 if (r["bridges_blank_lost"] or 0)>0: add("V2","WARNING",f"{r['bridges_blank_lost']} bridge(s) with blank LOST column")
 if r["evd_state"] in ("UNSCORED","GATED") and r["evd_balance"] is not None: add("V3","HARD FAIL","evd_balance set while UNSCORED/GATED")
 if rating is not None:
  if by.startswith("API") and rating>8: add("V4","HARD FAIL",f"API rating {rating} > 8")
  if by.startswith("HUMAN") and rating>9: add("V5","HARD FAIL",f"human rating {rating} > 9")
  if rating==10 and not by.startswith("LEAN"): add("V6","HARD FAIL","rating 10 without a Lean receipt")
 if rec["has_claims"] and not r["total_claims"]: add("V7","WARNING","no claims extracted")
 if r["truth_predicates"]==0: add("V8","WARNING","exhaustive extraction found no predicates")
 if None not in (r["lean_targets_queued"],r["total_claims"]) and r["lean_targets_queued"]>r["total_claims"]: add("V9","HARD FAIL","more Lean targets than claims")
 if None not in (r["claims_with_falsifiers"],r["total_claims"]) and r["claims_with_falsifiers"]>r["total_claims"]: add("V10","HARD FAIL","more claims with falsifiers than claims")
 ids=r["claim_ids"] if isinstance(r["claim_ids"],list) else []
 missing=[i for i in ids if rec["has_claims"] and i not in rec["claim_ids"]]
 if missing: add("V11","HARD FAIL","claim_ids not in the claims table: "+", ".join(map(str,missing)))
 # V12 needs the deferral section's question counts, which no station writes yet: not checked.
 return out

# ---------- rendering ----------
def inline(s:str)->str:
 s=html.escape(s)
 s=re.sub(r"`([^`]+)`",r"<code>\1</code>",s)
 s=re.sub(r"\*\*(.+?)\*\*",r"<strong>\1</strong>",s)
 return re.sub(r"(?<![*\w])\*(?!\s)(.+?)(?<!\s)\*(?!\w)",r"<em>\1</em>",s)

def markdown(text:str)->str:
 """Enough Markdown for the layers: headings, tables, lists, code fences, quotes, paragraphs. HTML comments are dropped."""
 text=re.sub(r"<!--.*?-->","",text,flags=re.S); out=[]; lines=text.replace("\r\n","\n").split("\n"); i=0
 while i<len(lines):
  s=lines[i].strip()
  if s.startswith("```"):
   j=i+1
   while j<len(lines) and not lines[j].strip().startswith("```"): j+=1
   out.append("<pre>"+html.escape("\n".join(lines[i+1:j]))+"</pre>"); i=j+1; continue
  if s.startswith("|"):
   rows=[]
   while i<len(lines) and lines[i].strip().startswith("|"): rows.append([c.strip() for c in lines[i].strip().strip("|").split("|")]); i+=1
   rows=[r for r in rows if not all(re.fullmatch(r":?-{2,}:?",c) for c in r)]
   if rows: out.append('<div class="scroll"><table><thead><tr>'+"".join(f"<th>{inline(c)}</th>" for c in rows[0])+"</tr></thead><tbody>"+
    "".join("<tr>"+"".join(f"<td>{inline(c)}</td>" for c in r)+"</tr>" for r in rows[1:])+"</tbody></table></div>")
   continue
  m=re.match(r"(#{1,6})\s+(.*)",s)
  if m: out.append(f"<h{min(6,len(m.group(1))+1)}>{inline(m.group(2))}</h{min(6,len(m.group(1))+1)}>"); i+=1; continue
  if re.match(r"([-*]|\d+\.)\s",s):
   tag="ol" if s[0].isdigit() else "ul"; items=[]
   while i<len(lines) and re.match(r"\s*([-*]|\d+\.)\s",lines[i]): items.append(re.sub(r"^\s*([-*]|\d+\.)\s","",lines[i])); i+=1
   out.append(f"<{tag}>"+"".join(f"<li>{inline(x)}</li>" for x in items)+f"</{tag}>"); continue
  if s.startswith(">"):
   q=[]
   while i<len(lines) and lines[i].strip().startswith(">"): q.append(lines[i].strip()[1:].strip()); i+=1
   out.append("<blockquote>"+inline(" ".join(q))+"</blockquote>"); continue
  if s in ("---","***"): out.append("<hr>"); i+=1; continue
  if not s: i+=1; continue
  p=[]
  while i<len(lines) and lines[i].strip() and not re.match(r"(#{1,6}\s|\||```|>|([-*]|\d+\.)\s)",lines[i].strip()): p.append(lines[i].strip()); i+=1
  out.append("<p>"+inline(" ".join(p))+"</p>")
 return "\n".join(out)

CSS=""":root{--bg:#0f1114;--panel:#171b21;--well:#0c0e12;--line:#343b45;--ink:#eef1f4;--muted:#a3aeba;--gold:#e6c069;--gold-dim:#8a7440;--green:#8bd5ba;--red:#ff9f99;
--serif:"Simula","Iowan Old Style",Georgia,serif;--sans:system-ui,-apple-system,"Segoe UI",sans-serif;--mono:"SF Mono","JetBrains Mono",ui-monospace,Menlo,monospace}
*{box-sizing:border-box}html{color-scheme:dark}body{margin:0;background:var(--bg);color:var(--ink);font:15px/1.6 var(--sans)}
header.top{border-bottom:1px solid var(--line);padding:22px 28px}header.top .kicker{color:var(--gold);font:12px var(--mono);letter-spacing:.14em;text-transform:uppercase}
header.top h1{font:400 30px/1.2 var(--serif);margin:6px 0 4px}header.top .sub{color:var(--muted);font-size:13px;overflow-wrap:anywhere}
.wrap{display:grid;grid-template-columns:240px minmax(0,1fr);gap:32px;padding:24px 28px 64px;max-width:1440px;margin:0 auto}
nav.rail{position:sticky;top:16px;align-self:start;max-height:calc(100vh - 32px);overflow:auto}nav.rail a{display:block;color:var(--muted);text-decoration:none;padding:6px 10px;border-radius:6px}
nav.rail a:hover{color:var(--ink);background:var(--panel)}nav.rail .label{color:var(--gold);font:11px var(--mono);letter-spacing:.12em;text-transform:uppercase;margin:16px 10px 4px}
main{min-width:0}section.block{margin:0 0 36px}h2{display:flex;align-items:center;gap:12px;font:400 22px/1.3 var(--serif);margin:0 0 6px}
h2 .n{border:1px solid var(--gold-dim);color:var(--gold);font:12px var(--mono);padding:2px 7px;border-radius:3px}h3,h4,h5,h6{font-family:var(--serif);font-weight:400}
.note{color:var(--muted);margin:0 0 10px}a{color:var(--gold)}.muted{color:var(--muted)}.scroll{overflow-x:auto}
table{border-collapse:collapse;width:100%;margin:8px 0;font-size:14px}th,td{border-bottom:1px solid var(--line);padding:7px 9px;text-align:left;vertical-align:top;overflow-wrap:anywhere}
th{background:var(--panel);color:var(--muted);font-weight:600;cursor:pointer;white-space:nowrap}tbody tr:hover{background:var(--panel)}
details{background:var(--panel);border:1px solid var(--line);border-radius:8px;margin:12px 0;padding:2px 18px}summary{cursor:pointer;font:600 15px var(--sans);padding:12px 0}
pre{white-space:pre-wrap;overflow-wrap:anywhere;background:var(--well);border:1px solid var(--line);padding:10px 12px;border-radius:6px;font:12.5px/1.5 var(--mono)}code{font-family:var(--mono);font-size:.9em;color:var(--gold)}
.chips span{display:inline-block;border:1px solid var(--line);border-radius:99px;padding:2px 11px;margin:2px 6px 2px 0;font-size:12.5px;color:var(--muted)}
.chips span.gold{border-color:var(--gold-dim);color:var(--gold)}.bad{color:var(--red)}.warn{color:var(--gold)}.ok{color:var(--green)}
input[type=search]{width:100%;max-width:440px;padding:9px 12px;border:1px solid var(--line);border-radius:6px;background:var(--well);color:var(--ink);font:inherit;margin:4px 0 20px}
.original{border-top:2px solid var(--gold-dim);margin-top:40px;padding-top:12px}.original .doc{font:17px/1.75 var(--serif);max-width:760px}
blockquote{border-left:3px solid var(--gold);margin:10px 0;padding:2px 14px;color:var(--muted);font-style:italic}hr{border:0;border-top:1px solid var(--line)}
@media (max-width:820px){.wrap{grid-template-columns:minmax(0,1fr);padding:16px}nav.rail{position:static;max-height:none;display:flex;flex-wrap:wrap;gap:4px}nav.rail .label{width:100%;margin:8px 0 0}header.top{padding:18px 16px}}"""
JS="""document.querySelectorAll('th').forEach(function(th){th.onclick=function(){var t=th.closest('table'),b=t.tBodies[0],i=[].indexOf.call(th.parentNode.children,th),d=th.dataset.d=th.dataset.d==='a'?'d':'a';
[].slice.call(b.rows).sort(function(x,y){var p=x.cells[i].innerText,q=y.cells[i].innerText,n=parseFloat(p)-parseFloat(q);var c=isNaN(n)?p.localeCompare(q):n;return d==='a'?c:-c}).forEach(function(r){b.appendChild(r)})}});
var f=document.getElementById('find');if(f)f.oninput=function(){var q=f.value.toLowerCase();document.querySelectorAll('tbody tr').forEach(function(r){r.style.display=r.innerText.toLowerCase().indexOf(q)<0?'none':''})};"""

def page(title:str,body:str,kicker:str="Theophysics / Papers",sub:str="",rail:list[tuple[str,str]]|None=None)->str:
 """rail: (href, text); an entry whose href is empty is a section label."""
 nav="".join(f'<div class="label">{html.escape(t)}</div>' if not h else f'<a href="{html.escape(h)}">{html.escape(t)}</a>' for h,t in rail or [])
 return (f'<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">'
  f'<title>{html.escape(title)}</title><style>{CSS}</style></head><body><header class="top"><div class="kicker">{html.escape(kicker)}</div>'
  f'<h1>{html.escape(title)}</h1><div class="sub">{sub}</div></header><div class="wrap"><nav class="rail">{nav}</nav><main>{body}</main></div>'
  f"<script>{JS}</script></body></html>")
def section(n,anchor,title,note,inner): return f'<section class="block" id="{anchor}"><h2><span class="n">{n:02d}</span>{html.escape(title)}</h2>'+(f'<p class="note">{html.escape(note)}</p>' if note else "")+inner+"</section>"
def body_only(text:str)->str:
 """The document without its leading front matter block (shown separately as a table)."""
 return re.sub(r"\A\s*---[ \t]*\n.*?\n---[ \t]*\n","",text,count=1,flags=re.S)
def rel(target:Path,start:Path)->str: return Path(os.path.relpath(target,start)).as_posix()

def cell(v)->str:
 if v is None: return '<span class="muted">—</span>'
 if isinstance(v,list): return html.escape(", ".join(map(str,v)))
 return html.escape(str(v))

def link(rec,frm:Path)->str:
 return f'<a href="{html.escape(rel(rec["paper"]/"index.html",frm))}">{html.escape(str(rec["row"]["clean_title"]))}</a>'

def table(recs:list[dict],cols:list[tuple[str,str]],frm:Path)->str:
 if not recs: return '<p class="muted">none</p>'
 head="".join(f"<th>{html.escape(h)}</th>" for h,_ in cols)
 body="".join("<tr>"+"".join(f"<td>{link(r,frm) if k=='_title' else cell(r['row'].get(k))}</td>" for _,k in cols)+"</tr>" for r in recs)
 return f'<div class="scroll"><table><thead><tr>{head}</tr></thead><tbody>{body}</tbody></table></div>'

def collection(rec):
 m=rec["meta"]
 return ("series",m["series"]) if m.get("series") else ("group",m["group"]) if m.get("group") else ("","")

def issues_html(issues):
 return "<ul>"+"".join(f'<li class="{"bad" if lv=="HARD FAIL" else "warn"}">{rule} {lv}: {html.escape(msg)}</li>' for rule,lv,msg in issues)+"</ul>"

def kv_table(d:dict)->str:
 return '<div class="scroll"><table><tbody>'+"".join(f"<tr><th>{html.escape(str(k))}</th><td>{cell(v)}</td></tr>" for k,v in d.items())+"</tbody></table></div>"

def paper_page(rec:dict,order:list[str],issues,home:Path)->str:
 """Written to <paper>/index.html: classification, validation, each layer in route order, then the original."""
 m,r,here=rec["meta"],rec["row"],rec["paper"]; kind,name=collection(rec)
 rank={n:i for i,n in enumerate(order)}
 stack=sorted(rec["stack"],key=lambda x:(rank.get(x[0][:2],len(order)),x[0]))
 chips="".join(f'<span class="{c}">{html.escape(x)}</span>' for c,x in [("gold",f"{kind}: {name}" if kind else ""),
  ("",f"order {m['order']}" if m.get("order") is not None else ""),("gold","priority" if m.get("priority") else ""),
  ("",f"{len(stack)} layer(s)"),("bad" if any(lv=="HARD FAIL" for _,lv,_ in issues) else "",f"{len(issues)} validation issue(s)" if issues else "")] if x)
 classified={k:v for k,v in r.items() if v not in (None,"",[]) and k!="clean_title" and not k.startswith("_")}
 body=f'<div class="chips">{chips}</div><input id="find" type="search" placeholder="Filter tables on this page">'
 n=1; body+=section(n,"classification","Classification","Every index field this paper has so far (MASTER_INDEX v0.3).",kv_table(classified))
 if issues: n+=1; body+=section(n,"validation","Validation","Listed, never fixed.",issues_html(issues))
 for label,run,text in stack:
  n+=1; shown=markdown(text) if (run/f"{label}.md").is_file() else "<pre>"+html.escape(text)+"</pre>"
  body+=section(n,label,label,f"latest run {run.name}",f'<details open><summary>Show / hide</summary>{shown}</details>')
 if not stack: n+=1; body+=section(n,"layers","Layers","No station has run on this paper yet.","")
 fm=frontmatter(rec["source"]); orig=(kv_table(fm) if fm else "")+f'<div class="doc">{markdown(body_only(rec["source"]))}</div>'
 body+=f'<section class="original" id="original"><h2><span class="n">{n+1:02d}</span>Original document</h2><p class="note">{html.escape(m["source_file"])} · 00_SOURCE (read-only)</p>{orig}</section>'
 rail=[("","This paper"),("#classification","Classification")]+([("#validation","Validation")] if issues else [])+[(f"#{l}",l) for l,_,_ in stack]+[("#original","Original document"),("","Navigate"),(rel(home/"index.html",here),"↑ All papers")]
 if kind: rail.append((rel(here.parent/"index.html",here),f"↑ {name}"))
 return page(str(r["clean_title"]),body,f"Theophysics / {kind or 'paper'}{' / '+name if kind else ''}",f"sha256 {html.escape(str(r['source_sha256']))}",rail)

def collection_page(kind:str,name:str,folder:Path,members:list[dict],problems:dict,home:Path)->str:
 """Written to <collection>/index.html: the papers in order, each station's folder, the latest series brief."""
 body='<input id="find" type="search" placeholder="Filter">'
 body+=section(1,"papers","Papers",f"{len(members)} in this {kind}"+(", in series order." if kind=="series" else "."),
  table(members,[("Paper","_title"),("Order","_order"),("Claims","total_claims"),("Domain","domain"),("Rating","paper_rating")],folder))
 by=folder/"_BY_STATION"; stations=sorted(p for p in by.iterdir() if p.is_dir()) if by.is_dir() else []
 body+=section(2,"by-station","By station","Each station's latest result for every paper, side by side.",
  "<ul>"+"".join(f'<li><a href="{html.escape(rel(st,folder))}/">{html.escape(st.name)}</a> <span class="muted">· {len(list(st.iterdir()))} files</span></li>' for st in stations)+"</ul>" if stations else '<p class="muted">none yet</p>')
 briefs=sorted((folder/"_SERIES").glob("*/48_SERIES_BRIEF.md"))
 body+=section(3,"brief","Series brief",f"latest of {len(briefs)}" if briefs else "Run station 48.",markdown(briefs[-1].read_text(encoding="utf-8",errors="replace")) if briefs else "")
 bad=[r for r in members if problems[r["paper"]]]
 body+=section(4,"validation","Validation","",("<ul>"+"".join(f"<li>{link(r,folder)}: "+"; ".join(f'<span class="{"bad" if lv=="HARD FAIL" else "warn"}">{a} {html.escape(m)}</span>' for a,lv,m in problems[r["paper"]])+"</li>" for r in bad)+"</ul>") if bad else '<p class="ok">All checks pass.</p>')
 rail=[("","This "+kind),("#papers","Papers"),("#by-station","By station"),("#brief","Series brief"),("#validation","Validation"),("","Navigate"),(rel(home/"index.html",folder),"↑ All papers")]
 return page(name,body,f"Theophysics / {kind}",f"{len(members)} papers",rail)

DASHBOARD=[
 ("Gated: resolve first","Papers with a verified counterexample open.",lambda r:r["evd_gated"] is True,"evd_counter",True,
  [("Paper","_title"),("Domain","domain"),("Gated / weakest","evd_weakest_claim"),("Support","evd_support"),("Counter","evd_counter"),("Families","evd_families"),("Rating","paper_rating")]),
 ("Unscored","Papers still at UNSCORED: how much of the question bank got answered.",lambda r:r["evd_state"]=="UNSCORED","evd_coverage",True,
  [("Paper","_title"),("Domain","domain"),("Coverage","evd_coverage"),("Families","evd_families"),("Type","content_type")]),
 ("Scored, weakest first","The one most likely to break is on top.",lambda r:r["evd_state"]=="SCORED","evd_balance",False,
  [("Paper","_title"),("Domain","domain"),("Balance","evd_balance"),("Weakest claim","evd_weakest_claim"),("Stable","evd_stable"),("Coverage","evd_coverage"),("Coherence","coherence"),("Rating","paper_rating")]),
 ("Scoring-dependent verdicts","Verdict flips under ±25% weights or cap ±1.",lambda r:r["evd_state"]=="SCORED" and r["evd_stable"] is False,"evd_balance",False,
  [("Paper","_title"),("Balance","evd_balance"),("Weakest claim","evd_weakest_claim"),("Domain","domain")]),
 ("Machine-only ratings","Rated by the API only: nothing here is admissible until a human ruling is recorded.",lambda r:str(r["rating_awarded_by"] or "").startswith("API"),"paper_rating",True,
  [("Paper","_title"),("Domain","domain"),("Rating","paper_rating"),("Awarded by","rating_awarded_by")]),
 ("Negative ratings","Failed tests, never deleted.",lambda r:(number(r["paper_rating"]) or 0)<0,"paper_rating",False,
  [("Paper","_title"),("Rating","paper_rating"),("Awarded by","rating_awarded_by"),("Domain","domain"),("Weakest claim","evd_weakest_claim")]),
]

def build(papers:list[Path]|None=None,order:list[str]|None=None)->Path:
 """Writes index.html into papers_root, every series/group folder and every paper folder; data files into papers_root/_SITE/."""
 home=external("papers_root"); data=home/"_SITE"; data.mkdir(parents=True,exist_ok=True)
 # The dashboard, collection pages and index.tsv always cover every paper; `papers` only picks which paper pages to rewrite.
 recs=[paper_record(p) for p in sorted(all_papers(home),key=run_order)]
 rewrite=None if papers is None else {Path(p).resolve() for p in papers}
 for r in recs: r["row"]["_order"]=r["meta"].get("order")
 order=order or []; problems={r["paper"]:validate(r) for r in recs}
 for rec in recs:
  if rewrite is None or rec["paper"] in rewrite: (rec["paper"]/"index.html").write_text(paper_page(rec,order,problems[rec["paper"]],home),encoding="utf-8")
 groups={}
 for rec in recs:
  k=collection(rec)
  if k[0]: groups.setdefault(k,[]).append(rec)
 for (kind,name),members in groups.items():
  folder=members[0]["paper"].parent; (folder/"index.html").write_text(collection_page(kind,name,folder,members,problems,home),encoding="utf-8")
 def tsv(v): return "" if v is None else ",".join(map(str,v)) if isinstance(v,list) else str(v).replace("\t"," ").replace("\n"," ")
 (data/"index.tsv").write_text("\t".join(COLUMNS)+"\n"+"".join("\t".join(tsv(r["row"][c]) for c in COLUMNS)+"\n" for r in recs),encoding="utf-8")
 (data/"validation.tsv").write_text("paper_id\trule\tlevel\tmessage\n"+"".join(f"{r['row']['paper_id']}\t{a}\t{b}\t{c}\n" for r in recs for a,b,c in problems[r["paper"]]),encoding="utf-8")
 stamp=datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC"); hard=sum(1 for v in problems.values() for x in v if x[1]=="HARD FAIL")
 body='<input id="find" type="search" placeholder="Filter every table">'; n=0; rail=[("","Dashboard")]
 if groups:
  n+=1; inner="".join(f'<details><summary>{html.escape(name)} <span class="muted">· {kind} · {len(m)} papers</span></summary><p><a href="{html.escape(rel(m[0]["paper"].parent/"index.html",home))}">Open the {kind} page</a></p><ol>'+"".join(f"<li>{link(r,home)}</li>" for r in m)+"</ol></details>" for (kind,name),m in sorted(groups.items()))
  body+=section(n,"collections","Series and groups","",inner); rail.append(("#collections","Series and groups"))
 for title,note,keep,key,desc,cols in DASHBOARD:
  rows=sorted((r for r in recs if keep(r["row"])),key=lambda r:(number(r["row"][key]) is None,(-1 if desc else 1)*(number(r["row"][key]) or 0)))
  n+=1; anchor=re.sub(r"[^a-z]+","-",title.lower()).strip("-"); body+=section(n,anchor,title,note,table(rows,cols,home)); rail.append((f"#{anchor}",title))
 failing=[r for r in recs if problems[r["paper"]]]
 n+=1; body+=section(n,"validation","Validation","Listed, never fixed. V12 is not checked yet.",("<ul>"+"".join(f"<li>{link(r,home)}: "+"; ".join(f'<span class="{"bad" if lv=="HARD FAIL" else "warn"}">{a} {html.escape(m)}</span>' for a,lv,m in problems[r["paper"]])+"</li>" for r in failing)+"</ul>") if failing else '<p class="ok">All checks pass.</p>')
 n+=1; body+=section(n,"every-paper","Every paper","",table(recs,[("Paper","_title"),("Domain","domain"),("Type","content_type"),("Rating","paper_rating"),("State","evd_state"),
  ("Claims","total_claims"),("Hidden premises","hidden_premises"),("Bridges","bridges_registered"),("Model","semantic_model")],home))
 rail+=[("#validation","Validation"),("#every-paper","Every paper"),("","Data"),("_SITE/index.tsv","index.tsv"),("_SITE/validation.tsv","validation.tsv")]
 body+='<p class="muted">Reads what the stations wrote. It cannot tell you a claim is true; it tells you which sheet to open next. POF 2828 · not a probability of truth · human ruling required.</p>'
 (home/"index.html").write_text(page("Papers",body,"Theophysics / Papers",f"{len(recs)} papers · {hard} hard validation failures · built {stamp}",rail),encoding="utf-8")
 return home/"index.html"
