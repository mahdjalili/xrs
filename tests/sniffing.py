"""Loopback-only Xray test, invoked by xray::tests::tls_sni_preserves_destination.

Send a real TLS ClientHello with SNI different from the SOCKS destination.
Verify domain routing and the destination passed to direct and proxy outbounds.
This tests TLS byte forwarding, not a completed TLS authentication handshake.
"""
import copy
import ipaddress
import json
import socket
import ssl
import subprocess
import sys
import tempfile
import time


def listener():
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    sock.listen()
    sock.settimeout(5)
    return sock


def read(sock, length):
    data = b""
    while len(data) < length:
        chunk = sock.recv(length - len(data))
        assert chunk, "unexpected EOF"
        data += chunk
    return data


def socks_connect(sock, address, port):
    sock.sendall(b"\x05\x01\x00")
    assert read(sock, 2) == b"\x05\x00"
    sock.sendall(b"\x05\x01\x00\x01" + socket.inet_aton(address) + port.to_bytes(2, "big"))
    assert read(sock, 4) == b"\x05\x00\x00\x01"
    read(sock, 6)


base = json.load(sys.stdin)
context = ssl.create_default_context()  # verification stays enabled
outgoing = ssl.MemoryBIO()
tls = context.wrap_bio(ssl.MemoryBIO(), outgoing, server_hostname="kube-teleport-proxy-alpn.teleport.example.com")
try:
    tls.do_handshake()
except ssl.SSLWantReadError:
    pass
hello = outgoing.read()
assert hello

for strategy in ("AsIs", "IPIfNonMatch"):
    for route in ("direct", "proxy"):
        with listener() as target, listener() as upstream, tempfile.TemporaryDirectory(prefix="xrs-sniffing-") as directory:
            # Obtain an unused loopback port for the core; startup retries below
            # report any bind race via its log and clean up the subprocess.
            with listener() as reservation:
                inbound_port = reservation.getsockname()[1]
            cfg = copy.deepcopy(base)
            cfg["inbounds"] = [cfg["inbounds"][0]]
            cfg["inbounds"][0]["listen"] = "127.0.0.1"
            cfg["inbounds"][0]["port"] = inbound_port
            cfg["outbounds"][0]["settings"]["servers"][0].update(address="127.0.0.1", port=upstream.getsockname()[1])
            cfg["routing"] = {
                "domainStrategy": strategy,
                "rules": [
                    {"type": "field", "domain": ["full:kube-teleport-proxy-alpn.teleport.example.com"], "outboundTag": route},
                    {"type": "field", "port": "0-65535", "outboundTag": "block"},
                ],
            }
            path = directory + "/config.json"
            with open(path, "w") as config_file:
                json.dump(cfg, config_file)
            with open(directory + "/core.log", "w+") as log:
                process = subprocess.Popen([sys.argv[1], "run", "-config", path], stdout=log, stderr=log)
                try:
                    deadline = time.monotonic() + 5
                    while True:
                        try:
                            client = socket.create_connection(("127.0.0.1", inbound_port), timeout=1)
                            break
                        except OSError:
                            assert process.poll() is None, "core exited during startup"
                            assert time.monotonic() < deadline, "core startup timed out"
                            time.sleep(0.02)
                    with client:
                        client.settimeout(5)
                        socks_connect(client, "127.0.0.1", target.getsockname()[1])
                        client.sendall(hello)
                        receiving = target if route == "direct" else upstream
                        peer, _ = receiving.accept()
                        with peer:
                            peer.settimeout(5)
                            if route == "proxy":
                                assert read(peer, 1) == b"\x05"
                                methods = read(peer, read(peer, 1)[0])
                                assert 0 in methods
                                peer.sendall(b"\x05\x00")
                                assert read(peer, 3) == b"\x05\x01\x00"
                                kind = read(peer, 1)[0]
                                assert kind == 1, "proxy must receive original IP, not SNI"
                                assert ipaddress.ip_address(read(peer, 4)) == ipaddress.ip_address("127.0.0.1")
                                assert int.from_bytes(read(peer, 2), "big") == target.getsockname()[1]
                                peer.sendall(b"\x05\x00\x00\x01\x7f\x00\x00\x01\x00\x00")
                            assert read(peer, len(hello)) == hello, "TLS bytes must remain unchanged"
                            peer.sendall(b"forwarded")
                            assert read(client, 9) == b"forwarded"
                    print(f"PASS: {strategy}, {route}: SNI routed, original destination preserved", flush=True)
                except BaseException:
                    log.flush()
                    log.seek(0)
                    print(log.read(), file=sys.stderr)
                    raise
                finally:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
