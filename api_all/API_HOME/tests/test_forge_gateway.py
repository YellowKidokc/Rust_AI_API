import json,os,threading,unittest
from http.server import BaseHTTPRequestHandler,HTTPServer
from pathlib import Path
from unittest.mock import patch
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from engine import llm

class _Fake(BaseHTTPRequestHandler):
 seen=[]
 def do_POST(self):
  body=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
  _Fake.seen.append((self.path,body,self.headers.get('Authorization')))
  out=json.dumps({'choices':[{'message':{'content':'ok from forge'}}],'usage':{'prompt_tokens':3,'completion_tokens':2}}).encode()
  self.send_response(200); self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(out))); self.end_headers(); self.wfile.write(out)
 def log_message(self,*_): pass

class ForgeGatewayTests(unittest.TestCase):
 def setUp(self):
  _Fake.seen.clear(); self.server=HTTPServer(('127.0.0.1',0),_Fake)
  threading.Thread(target=self.server.serve_forever,daemon=True).start()
  self.base=f'http://127.0.0.1:{self.server.server_port}/'
 def tearDown(self): self.server.shutdown(); self.server.server_close()
 def test_forge_url_redirects_without_provider_key(self):
  env={'FORGE_URL':self.base,'DEEPSEEK_API_KEY':''}
  with patch.dict(os.environ,env): os.environ.pop('FORGE_ROUTE',None); os.environ.pop('FORGE_TOKEN',None); r=llm.call([{'role':'user','content':'hi'}])
  self.assertIsNone(r.error); self.assertEqual('ok from forge',r.text); self.assertEqual(5,r.prompt_tokens+r.completion_tokens)
  path,body,auth=_Fake.seen[0]
  self.assertEqual('/v1/chat/completions',path); self.assertEqual('deepseek-chat',body['model']); self.assertIsNone(auth)
 def test_forge_route_and_token(self):
  with patch.dict(os.environ,{'FORGE_URL':self.base,'FORGE_ROUTE':'free','FORGE_TOKEN':'t0k'}): llm.call([{'role':'user','content':'hi'}],provider='openai',model='gpt-5-mini')
  _,body,auth=_Fake.seen[0]; self.assertEqual('free',body['model']); self.assertEqual('Bearer t0k',auth)
 def test_no_forge_url_keeps_direct_path(self):
  with patch.dict(os.environ,{'FORGE_URL':''}): self.assertIsNone(llm._forge_target('deepseek-chat'))

if __name__=='__main__': unittest.main()
