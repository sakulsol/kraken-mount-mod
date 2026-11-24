#!/usr/bin/env python3
# Windows fixes for BlazingMount's host scripts (applied to a clean checkout in CI):
#  1. dirserve.py: os.pread does not exist on Windows -> seek + read on the open file.
#  2. blazingmount-install.py: verify each pushed file by reading it back over the console's FTP
#     (the same server it was written with), not over a websrv on :8080 the console may not run.
import sys

def sub(path, old, new):
    s = open(path, encoding="utf-8").read()
    if s.count(old) != 1:
        sys.exit("patch anchor not found exactly once in %s: %r" % (path, old[:80]))
    open(path, "w", encoding="utf-8", newline="").write(s.replace(old, new))

root = sys.argv[1]
sub(root + "/host/serve/dirserve.py",
    "                        buf = os.pread(fd, min(CHUNK, length - sent), start + sent)\n",
    "                        fh.seek(start + sent)\n"
    "                        buf = fh.read(min(CHUNK, length - sent))\n")
sub(root + "/host/serve/dirserve.py",
    "            except (BrokenPipeError, ConnectionResetError):\n                pass\n        ms = int",
    "            except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):\n                pass\n        ms = int")
sub(root + "/install/blazingmount-install.py",
    'def websrv_sha256(console, remote):\n    url = "http://%s:8080/fs%s" % (console, remote)\n    h = hashlib.sha256()\n    with urllib.request.urlopen(url, timeout=120) as r:\n        for b in iter(lambda: r.read(1 << 20), b""):\n            h.update(b)\n    return h.hexdigest()\n',
    'FTP_PORT = 2121\n\n\ndef websrv_sha256(console, remote):\n    # Windows build: read the file back over the console FTP (no websrv on :8080 needed).\n    h = hashlib.sha256()\n    ftp = ftp_connect(console, FTP_PORT)\n    ftp.retrbinary("RETR " + remote, h.update)\n    ftp.quit()\n    return h.hexdigest()\n')
sub(root + "/install/blazingmount-install.py",
    "    a = ap.parse_args()\n",
    "    a = ap.parse_args()\n    global FTP_PORT\n    FTP_PORT = a.ftp_port\n")
print("windows patches applied")
