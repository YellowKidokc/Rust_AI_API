from __future__ import annotations
import json,re,shutil
from datetime import datetime,timezone
from pathlib import Path
from .output import sha256_file
from .paths import external,inside
from .tagger import tag_item

def slugify(text: str) -> str:
    return re.sub(r"[^a-z0-9]+","-",text.lower()).strip("-")[:80] or "untitled"

# Papers live loose under papers_root, or together in a collection:
#   papers_root/SERIES/<name>/<NN>_<ID>_<slug>/   a series: ordered; gets a series brief (station 48)
#   papers_root/GROUPS/<name>/<ID>_<slug>/        a group: kept together, no order
# Collection-level outputs sit beside the papers in _BY_STATION/ and _SERIES/, which hold no paper.json.
KINDS={"series":"SERIES","group":"GROUPS"}
SOURCE_TYPES={".md",".txt",".markdown",".html",".htm"}

def collection_dir(series: str="", group: str="") -> Path:
    root=external("papers_root")
    if series and group: raise ValueError("A paper is in a series or a group, not both")
    if series: return root/"SERIES"/series
    if group: return root/"GROUPS"/group
    return root

def order_of(name: str) -> int|None:
    m=re.match(r"\s*(\d+)",name); return int(m.group(1)) if m else None

def all_papers(root: Path|None=None) -> list[Path]:
    root=root or external("papers_root")
    return sorted({p.parent.resolve() for p in root.rglob("paper.json") if not {"_BY_STATION","_SERIES"} & set(p.relative_to(root).parts)})

def hash_index() -> dict[str,Path]:
    """source SHA-256 -> paper folder, for every paper under papers_root."""
    root=external("papers_root"); index={}
    if not root.is_dir(): return index
    for paper in all_papers(root):
        try: index.setdefault(json.loads((paper/"paper.json").read_text(encoding="utf-8")).get("source_hash"),paper)
        except (OSError, json.JSONDecodeError): continue
    return index

def create(source: Path, *, title: str|None=None, paper_id: str|None=None, series: str="", group: str="",
           order: int|None=None, priority: bool=False, known: dict[str,Path]|None=None) -> Path:
    """Make the paper's working folder. The same source (by SHA-256) is never made twice: its existing folder is returned."""
    source=source.expanduser().resolve()
    if not source.is_file(): raise FileNotFoundError(source)
    digest=sha256_file(source)
    known=hash_index() if known is None else known
    if digest in known: return known[digest]
    if series and order is None: order=order_of(source.name)
    # In a series the leading number is the order, not part of the title ("01 God Is.md" -> "God Is").
    title=title or (re.sub(r"^\s*\d+[\s._-]*","",source.stem) if series and order is not None else source.stem) or source.stem
    paper_id=paper_id or digest[:12].upper(); name=f"{paper_id}_{slugify(title)}"
    if series and order is not None: name=f"{order:02d}_{name}"
    destination=collection_dir(series,group)/name
    if destination.exists(): raise FileExistsError(f"Paper folder already exists: {destination}")
    destination.parent.mkdir(parents=True,exist_ok=True)
    shutil.copytree(inside("templates","PAPER_FOLDER"),destination)
    copied=destination/"00_SOURCE"/source.name; shutil.copy2(source,copied)
    copied.chmod(0o444); (destination/"00_SOURCE"/"sha256.txt").write_text(f"{digest}  {source.name}\n",encoding="utf-8")
    paper={"id":paper_id,"title":title,"series":series,"group":group,"order":order,"priority":priority,"status":"new","source_hash":digest,
           "source_file":source.name,"source_path":str(source),"created_at":datetime.now(timezone.utc).isoformat(),"stations_run":[],"headline_scores":{},"tags":{}}
    (destination/"paper.json").write_text(json.dumps(paper,indent=2),encoding="utf-8"); known[digest]=destination
    # Tagging is automatic when a catalog database is configured; creation remains
    # usable offline when it is intentionally blank.
    try:
        if external("catalog_db", required=False) != Path():
            values=tag_item(paper_id,"paper",destination,source.read_text(encoding="utf-8",errors="replace"))
            paper["tags"]={x["tag"]:x for x in values}
            (destination/"paper.json").write_text(json.dumps(paper,indent=2),encoding="utf-8")
    except (OSError, ValueError, json.JSONDecodeError):
        pass
    return destination

def _sources(folder: Path) -> list[Path]:
    return sorted(p for p in folder.iterdir() if p.is_file() and p.suffix.lower() in SOURCE_TYPES and not p.name.startswith((".","~")))

def intake(folder: Path, *, series: str="", group: str="", priority: bool=False, known: dict[str,Path]|None=None) -> list[Path]:
    """Make paper folders for everything in `folder`, reading files in place (originals are never moved).
    Inside it: 00_PRIORITY/ is taken first and marked priority; SERIES/<name>/ and GROUPS/<name>/ set the collection."""
    folder=folder.expanduser().resolve(); made=[]
    if not folder.is_dir(): raise NotADirectoryError(folder)
    known=hash_index() if known is None else known
    if (folder/"00_PRIORITY").is_dir(): made+=intake(folder/"00_PRIORITY",series=series,group=group,priority=True,known=known)
    for kind,dirname in KINDS.items():
        if (folder/dirname).is_dir():
            for sub in sorted(x for x in (folder/dirname).iterdir() if x.is_dir()):
                made+=intake(sub,priority=priority,known=known,**{kind:sub.name})
    for source in _sources(folder): made.append(create(source,series=series,group=group,priority=priority,known=known))
    return made

def _meta(paper: Path) -> dict:
    try: return json.loads((paper/"paper.json").read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError): return {}

def run_order(paper: Path) -> tuple:
    """Priority first; then each collection kept together, a series in its own order."""
    m=_meta(paper); o=m.get("order")
    return (not m.get("priority"), m.get("series") or "", m.get("group") or "", o if isinstance(o,int) else 10**9, paper.name)

def discover(items:list[str], limit:int|None=None)->list[Path]:
    result=[]
    for item in items:
        p=Path(item).expanduser()
        if p.is_dir() and (p/"paper.json").is_file(): result.append(p.resolve())
        elif p.is_dir() and all_papers(p): result+=all_papers(p)   # a collection or the papers root
        elif p.is_file(): result.append(p.resolve())
    if not items:
        result=all_papers()
    papers=sorted({x for x in result if x.is_dir()},key=run_order); files=[x for x in result if not x.is_dir()]
    result=papers+files
    return result[:limit] if limit is not None else result
