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
# 3. mkgame.py: the public repo lacks decrypt_fself (external) and mkbackport.py (its .handoff says so). They only
#    serve to read the eboot's DT_NEEDED and import NIDs; in a fake-signed SELF (scene dumps) and a plain ELF the
#    dynamic string table is plaintext, so without them the names are found by scanning the eboot's bytes.
sub(root + "/host/engine/mkgame.py",
    "        sys.path.insert(0, os.path.join(BACKPORT_TOOLS, \"src\"))\n        from decrypt_fself import UnsignedELFConverter\n        return UnsignedELFConverter().convert_file(raw, dst)\n",
    "        sys.path.insert(0, os.path.join(BACKPORT_TOOLS, \"src\"))\n        try:\n            from decrypt_fself import UnsignedELFConverter\n        except ImportError:\n            shutil.copyfile(raw, dst)   # Windows build: no decrypt_fself; elf_needs scans the SELF's plaintext strings\n            return True\n        return UnsignedELFConverter().convert_file(raw, dst)\n")
sub(root + "/host/engine/mkgame.py",
    "    from mkbackport import Dyn  # same folder as this file\n    data = open(path, \"rb\").read()\n",
    "    data = open(path, \"rb\").read()\n    try:\n        from mkbackport import Dyn  # same folder as this file\n    except ImportError:\n        # Windows build: no mkbackport.py. Library names and NID#lib#mod import strings from the raw bytes.\n        names = sorted(set(m.decode() for m in re.findall(rb\"lib[A-Za-z0-9_]+\\.(?:prx|sprx)\", data)))\n        nids = set(m.decode() for m in re.findall(rb\"([A-Za-z0-9+\\-_]{11})#[A-Za-z0-9]{1,3}#[A-Za-z0-9]{1,3}\", data))\n        return names, nids\n")
open(root + "/WINDOWS-PATCH", "w").write("3\n")
print("windows patches applied")
