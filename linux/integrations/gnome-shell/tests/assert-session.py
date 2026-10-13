import ast, base64, json, os, subprocess, time
path = os.environ['XDG_RUNTIME_DIR']
def call(method, *args):
    result = subprocess.run(['gdbus','call','--session','--dest','org.kebloop.Gnome','--object-path','/org/kebloop/Gnome','--method','org.kebloop.Gnome1.'+method,*map(str,args)], capture_output=True, text=True, timeout=4, check=True)
    return result.stdout.strip()
def snapshot(method='GetClipboard'):
    output = call(method, 'true') if method == 'GetClipboard' else call(method)
    return json.loads(ast.literal_eval(output)[0])
def wait(check):
    for _ in range(100):
        value = check()
        if value: return value
        time.sleep(.05)
    raise AssertionError('condition timed out')
def windows(): return snapshot('Windows')
def command(action, **extra): return call('Command', json.dumps(dict(action=action,id=window_id,**extra)))
assert call('Ping') == '(uint32 1,)'
assert snapshot('Outputs')[0]['usable']['h'] < 720
log = open(path+'/entry.log','w')
client = subprocess.Popen(['gjs','-m',os.path.join(os.path.dirname(__file__), 'entry.mjs')],stdout=log,stderr=log)
window = wait(lambda: next((w for w in windows() if w['title']=='KebloopPasteSmoke'),None))
assert window['app_id'] == 'app.kebloop.Smoke', window
window_id=window['id']
call('Paste','false') # Create headless seat keyboard before requesting keyboard focus.
command('focus')
wait(lambda: next(w for w in windows() if w['id']==window_id)['focused'])
command('frame',frame=dict(x=20,y=80,w=500,h=300))
wait(lambda: next(w for w in windows() if w['id']==window_id)['frame']==dict(x=20,y=80,w=500,h=300))
command('hide')
wait(lambda: next(w for w in windows() if w['id']==window_id)['hidden'])
command('focus')
wait(lambda: not next(w for w in windows() if w['id']==window_id)['hidden'])
snapshot() # Enable recording.
text=b'GNOME native paste smoke'
subprocess.run(['wl-copy','--type','text/plain'],input=text,check=True,timeout=4)
wait(lambda: snapshot()['ready'])
assert base64.b64decode(snapshot('GetClipboardData')['data']['text/plain;charset=utf-8'])==text
call('SetClipboard','text/plain',base64.b64encode(text).decode(),'false')
assert subprocess.run(['wl-paste','--no-newline'],capture_output=True,check=True,timeout=4).stdout == text
assert snapshot()['own_write']
call('Paste','false')
wait(lambda: 'ENTRY:GNOME native paste smoke' in open(path+'/entry.log').read())
# Paused recording never requests/caches clipboard data.
call('GetClipboard','false')
subprocess.run(['wl-copy','--type','text/plain'],input=b'paused',check=True,timeout=4)
assert json.loads(ast.literal_eval(call('GetClipboard','false'))[0])['data']=={}
assert snapshot('GetClipboardData')['data']=={}
snapshot()
wait(lambda: bool(snapshot('GetClipboardData')['data']))
assert base64.b64decode(snapshot('GetClipboardData')['data']['text/plain;charset=utf-8'])==b'paused'
# Password marker produces metadata only.
subprocess.run(['wl-copy','--type','x-kde-passwordManagerHint'],input=b'secret',check=True,timeout=4)
assert snapshot()['sensitive']
assert snapshot('GetClipboardData')['data']=={}
# Exact PNG memory-source round trip.
png=base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aPioAAAAASUVORK5CYII=')
call('SetClipboard','image/png',base64.b64encode(png).decode(),'false')
assert subprocess.run(['wl-paste','--type','image/png','--no-newline'],capture_output=True,check=True,timeout=4).stdout == png
wait(lambda: snapshot()['ready'])
assert base64.b64decode(snapshot('GetClipboardData')['data']['image/png'])==png
before=snapshot()['revision']
call('ClearClipboard')
assert snapshot()['revision']>before
assert snapshot()['mimes']==[]
command('close')
wait(lambda: all(w['id']!=window_id for w in windows()))
client.wait(timeout=4)
print('GNOME real Wayland assertions passed: geometry/focus/minimize/close, text/history/native paste, pause/resume, sensitive marker, PNG and clear revision')
