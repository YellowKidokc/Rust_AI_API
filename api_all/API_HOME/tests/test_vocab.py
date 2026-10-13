import json,os,sys,tempfile,threading,unittest
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from engine import paper_station
from engine.papers import create

class _Model(BaseHTTPRequestHandler):
 prompts=[]
 def do_POST(self):
  _Model.prompts.append(json.loads(self.rfile.read(int(self.headers['Content-Length'])))['messages'][-1]['content'])
  reply='|C001|`cl-causal`|Grace restores order.|\n|C002|`cl-made-up`|Invented.|\n|C003|`UNCLASSIFIED`|No fit.|'
  out=json.dumps({'choices':[{'message':{'content':reply}}],'usage':{'prompt_tokens':1,'completion_tokens':1}}).encode()
  self.send_response(200); self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(out))); self.end_headers(); self.wfile.write(out)
 def log_message(self,*_): pass

class VocabularyTests(unittest.TestCase):
 def test_vocab_goes_first_is_shared_and_codes_are_checked(self):
  server=ThreadingHTTPServer(('127.0.0.1',0),_Model); threading.Thread(target=server.serve_forever,daemon=True).start()
  try:
   with tempfile.TemporaryDirectory() as td:
    root=Path(td); (root/'papers').mkdir(); (root/'paths.json').write_text(json.dumps({'papers_root':str(root/'papers'),'catalog_db':''}))
    vocab=root/'claim_types.md'; vocab.write_text('| Code | Meaning |\n|---|---|\n| `cl-causal` | X causes Y |\n| `cl-definitional` | X is Y by definition |\n')
    env={'ONE_MENU_PATHS_FILE':str(root/'paths.json'),'FORGE_URL':f'http://127.0.0.1:{server.server_port}','FORGE_ROUTE':'test'}
    with patch.dict(os.environ,env):
     papers=[]
     for i in range(2):
      src=root/f'p{i}.md'; src.write_text(f'Paper {i}.'); papers.append(str(create(src)))
     with patch.object(sys,'argv',['x',*papers,'--vocab',str(vocab)]): paper_station.run_ai('49_CLAIMS_LAYER',[('claims','Find the claims.')])
    a,b=_Model.prompts
    self.assertTrue(a.startswith('REFERENCE VOCABULARY'))
    head=a.index('END OF REFERENCE VOCABULARY'); self.assertEqual(a[:head],b[:head])  # identical opening for every paper
    self.assertIn('`cl-definitional`',a)                                             # the whole vocabulary is sent
    run=json.loads(next(Path(papers[0],'02_RUNS','49_CLAIMS_LAYER').glob('*/49_CLAIMS_LAYER.run.json')).read_text())
    self.assertEqual({'cl-causal':1,'UNCLASSIFIED':1},run['calls'][0]['codes_used'])
    self.assertEqual(['cl-made-up'],run['codes_unknown'])
    self.assertIn('vocab_hash',run)
  finally: server.shutdown(); server.server_close()

if __name__=='__main__': unittest.main()
