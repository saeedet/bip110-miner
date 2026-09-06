#!/usr/bin/env python3
"""A reference implementation of the BLAKE2b-fork proof of work.

This is not part of the miner. It exists because the proof of work is a
five-stage pipeline that is easy to transcribe *almost* correctly, and a
version that can be run and diffed against a live node is worth more than
prose. It is the ground truth the Rust implementation is tested against.

Verified against every BLAKE2b block on a local regtest chain.

    ./scripts/node.sh cli regtest getblockheader <hash> false \\
      | xargs python3 scripts/pow_reference.py

One subtlety worth naming, because it cost a debugging cycle: the C++ writes
`final_hash` backwards (`*--out`), producing the value in *internal* order,
and the RPC then reverses it again for display. So the displayed block hash
is the masked digest as-is, with no reversal. Reversing once gets you a hash
that looks plausible and is exactly wrong -- the same trap as Bitcoin's
display convention, one layer further down.

See docs/proof-of-work.md for what each stage is for.
"""
import hashlib, struct, sys
def sha256(b): return hashlib.sha256(b).digest()
def tagged(tag, data):
    t = sha256(tag.encode() if isinstance(tag, str) else tag)
    return sha256(t + t + data)
def blake2b32(b): return hashlib.blake2b(b, digest_size=32).digest()

def pow_hash(hdr: bytes) -> str:
    o = 0
    def take(n):
        nonlocal o
        b = hdr[o:o+n]; o += n; return b
    u32 = lambda: struct.unpack("<I", take(4))[0]
    v            = u32();  prev = take(32); merkle = take(32)
    time_on_wire = u32();  nbits = u32();   nonce  = u32()
    nonce2       = u32();  nonce3 = u32();  extranonce = take(16)
    time_offset  = u32()
    txcount      = struct.unpack("<H", take(2))[0]
    flags        = take(1)[0]; xor_clear = take(1)[0]
    xor_key      = take(16);   height = u32(); mm_rhs = take(32)
    assert o == 164

    xor_key_hash = tagged("Bitcoin block hash PoW XOR key", xor_key)
    xor_mask = bytes(32)
    if any(xor_key):
        m = bytearray(tagged("Bitcoin block hash PoW XOR mask", xor_key))
        cb = xor_clear // 8
        for i in range(cb): m[i] = 0
        m[cb] &= 0xFF >> (xor_clear % 8)
        xor_mask = bytes(m)

    prev_sane   = prev[::-1]
    prev_hidden = tagged("Bitcoin prevblock header, hashed", prev_sane)

    h1_body = (struct.pack("<I", v) + prev_sane + struct.pack("<I", height) + merkle
               + struct.pack("<I", time_on_wire) + b"\x00" + struct.pack("<I", nbits)
               + struct.pack("<I", txcount) + bytes([flags, xor_clear]) + xor_key_hash)
    assert len(h1_body) == 119
    h1 = tagged("Bitcoin block header 1", h1_body)
    h2 = tagged("Merge-mining hook", h1 + bytes(16) + bytes(16) + mm_rhs)

    ss = struct.pack("<I", 0) + h2 + extranonce
    assert len(ss) == 52
    h = blake2b32(ss)

    n, n2 = struct.pack("<I", nonce), struct.pack("<I", nonce2)
    n3, to = struct.pack("<I", nonce3), struct.pack("<I", time_offset)
    case = flags & 3
    if case == 0:
        s = bytearray(prev_hidden); s[:6] = bytes(6)
        ss = bytes(s) + n + n2 + to + n3 + h
    elif case == 1:
        ss = n + n2 + n3 + to + h + h2
    else:
        ss = (bytes(32) if case == 3 else b"") + bytes(48) + h2 + n + n2 + to + n3 + h
    h = blake2b32(ss)

    # The C++ writes final_hash backwards, giving INTERNAL order; the RPC then
    # reverses it again for display. Display order is therefore the masked
    # digest as-is.
    return bytes(h[i] ^ xor_mask[i] for i in range(32)).hex()

if __name__ == "__main__":
    print(pow_hash(bytes.fromhex(sys.argv[1])))
