import json,os,sys,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from engine import site
from engine.papers import intake

SOURCE="""---
type: evidence-sheet
paper_id: "law9-noether"
clean_title: "Law 9 and Noether"
domain: Physics
content_type: ct-argument
paper_rating: 9
rating_awarded_by: API
evd_state: SCORED
evd_balance: -0.4
evd_gated: true
evd_counter: 3
evd_stable: false
claim_ids: [L9-C001, L9-C999]
---
# Law 9 and Noether
> **Extraction completeness check** 12 predicates extracted from 30 source paragraphs.

**Supplementary argument 1** time translation.
"""
CLAIMS="""```yaml
---
type: claims_layer
source_sha256: {sha}
semantic_provider: "deepseek"
semantic_model: "deepseek-chat"
---
```
## Claims table
|ID|Version|Register|Text|
|---|---|---|---|
|L9-C001|v1|NATIVE|Moral energy is conserved.|
|L9-C002|v1|BRIDGE|Parity breaks.|

## Layer integrity
| Check | Value |
|---|---|
| Total claims | 2 |
| Claims with falsifiers | 2 |
| Hidden premises found | 1 |
| Lean targets queued | 3 |
| Predictions logged | 0 |
| Bridges registered | 1 |
| Bridges with blank LOST column | 0 |
"""

class SiteTests(unittest.TestCase):
 def test_index_validation_and_stacked_page(self):
  with tempfile.TemporaryDirectory() as td:
   root=Path(td); inbox=root/'INBOX'; (root/'papers').mkdir()
   (inbox/'SERIES'/'God Is').mkdir(parents=True); (inbox/'SERIES'/'God Is'/'01 Law 9.md').write_text(SOURCE)
   (inbox/'SERIES'/'God Is'/'02 Grace.md').write_text('# Grace\nGrace restores order.')
   (root/'paths.json').write_text(json.dumps({'papers_root':str(root/'papers'),'catalog_db':''}))
   with patch.dict(os.environ,{'ONE_MENU_PATHS_FILE':str(root/'paths.json')}):
    first,second=intake(inbox); sha=json.loads((first/'paper.json').read_text())['source_hash']
    for label,name,text in [('49_CLAIMS_LAYER','49_CLAIMS_LAYER.md',CLAIMS.format(sha=sha)),('40_ANALYTICAL_ARMS','40_ANALYTICAL_ARMS.json','{"coherence": "fine"}')]:
     run=first/'02_RUNS'/label/'20261013T000000Z'; run.mkdir(parents=True); (run/name).write_text(text)
    out=site.build(order=['49','40'])
   data=out.parent/'_SITE'; self.assertEqual('index.html',out.name)
   header,*rows=(data/'index.tsv').read_text().splitlines()
   cols=header.split('\t'); self.assertEqual(43,len(cols)); self.assertEqual(2,len(rows))
   row=dict(zip(cols,rows[0].split('\t')))
   self.assertEqual('law9-noether',row['paper_id']); self.assertEqual(sha,row['source_sha256'])
   self.assertEqual('2',row['total_claims']); self.assertEqual('3',row['lean_targets_queued'])
   self.assertEqual('12',row['truth_predicates']); self.assertEqual('30',row['source_paragraphs']); self.assertEqual('1',row['supplementary_arguments'])
   self.assertEqual('deepseek-chat',row['semantic_model'])
   rules={l.split('\t')[1] for l in (data/'validation.tsv').read_text().splitlines()[1:]}
   self.assertEqual({'V4','V9','V11'},rules)          # API rating 9 > 8; 3 Lean targets > 2 claims; L9-C999 dangling
   page=(first/'index.html').read_text()                                             # a page in the paper's own folder
   self.assertLess(page.index('49_CLAIMS_LAYER'),page.index('40_ANALYTICAL_ARMS'))   # route order
   self.assertLess(page.index('40_ANALYTICAL_ARMS'),page.index('Original document'))  # layers on top, original below
   self.assertIn('<td>Moral energy is conserved.</td>',page)                         # markdown tables rendered
   dash=out.read_text(); series_page=(first.parent/'index.html').read_text()
   self.assertIn('01_',series_page); self.assertIn('Grace',series_page)
   self.assertNotIn('type: evidence-sheet paper_id',page)                             # front matter shown as a table, not a run-on paragraph
   self.assertIn('God Is',dash); self.assertIn('Gated: resolve first',dash)
   self.assertIn('Law 9 and Noether</a>',dash.split('id="gated-resolve-first"')[1].split('</section>')[0])   # gated paper in the Gated section

if __name__=='__main__': unittest.main()
