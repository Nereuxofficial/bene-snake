"""Test-only production HTTP fixtures. Labels are used only by sensitivity mocks."""
import sys, os, json, time, subprocess, fcntl
from http.server import HTTPServer, BaseHTTPRequestHandler
mode = sys.argv[1]
fixed = sys.argv[2]
answers = json.load(open(sys.argv[3])) if len(sys.argv)>3 and sys.argv[3]!='-' else []
open('parent.pid','w').write(str(os.getpid()))
lock = open('candidate.lock','a')
try: fcntl.flock(lock, fcntl.LOCK_EX|fcntl.LOCK_NB)
except BlockingIOError: sys.exit(78)
if mode == 'hung_startup': time.sleep(60)
if mode == 'descendant':
    child = subprocess.Popen(['/usr/bin/sleep','60'])
    open('descendant.pid','w').write(str(child.pid))
class Handler(BaseHTTPRequestHandler):
    def log_message(self,*args): pass
    def do_GET(self):
        self.send_response(200);self.send_header('Content-Length','2');self.end_headers();self.wfile.write(b'{}')
    def do_POST(self):
        r=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        if self.path=='/end' and mode=='hung_end': time.sleep(60)
        if self.path=='/move':
            local_mode=mode
            for answer in answers:
                if answer['board']==r['board']: local_mode=answer.get('mode',mode);break
            if local_mode=='crash': os._exit(3)
            if local_mode=='late': time.sleep(.2)
            if local_mode=='http_error': self.send_error(500);return
            if local_mode=='malformed': data=b'{"move":"north"}'
            elif local_mode=='oversized': data=b'x'*100000
            else:
                chosen=fixed
                for a in answers:
                    if a['board']==r['board']:
                        chosen=a['success'] if mode=='good' else a['failure'];break
                data=json.dumps({'move':chosen}).encode()
            self.send_response(200);self.send_header('Content-Length',str(len(data)));self.end_headers()
            if local_mode=='partial': self.wfile.write(data[:3]);self.wfile.flush();time.sleep(.2);data=data[3:]
            self.wfile.write(data);return
        self.send_response(200);self.send_header('Content-Length','2');self.end_headers();self.wfile.write(b'{}')
HTTPServer(('127.0.0.1',int(os.environ['PORT'])),Handler).serve_forever()
