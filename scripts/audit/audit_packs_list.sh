#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PACKS_DIR="${ROOT_DIR}/packs"
EVIDENCE_DIR="${ROOT_DIR}/docs/audit/packs/_evidence"
MANIFEST_DIR="${EVIDENCE_DIR}/manifests"
LOCK_DIR="${EVIDENCE_DIR}/locks"

mkdir -p "${MANIFEST_DIR}" "${LOCK_DIR}"

packs_list="${EVIDENCE_DIR}/packs_list.txt"
rm -f "${packs_list}"

python3 - <<'PY' "${PACKS_DIR}" "${MANIFEST_DIR}" "${packs_list}"
import json
import sys
import zipfile
from pathlib import Path

packs_dir = Path(sys.argv[1])
manifest_dir = Path(sys.argv[2])
packs_list_path = Path(sys.argv[3])


class CBORDecoder:
    def __init__(self, data: bytes):
        self.data = data
        self.pos = 0

    def read(self, n: int) -> bytes:
        if self.pos + n > len(self.data):
            raise ValueError("truncated CBOR input")
        chunk = self.data[self.pos : self.pos + n]
        self.pos += n
        return chunk

    def decode_uint(self, addl: int) -> int:
        if addl < 24:
            return addl
        if addl == 24:
            return self.read(1)[0]
        if addl == 25:
            return int.from_bytes(self.read(2), "big")
        if addl == 26:
            return int.from_bytes(self.read(4), "big")
        if addl == 27:
            return int.from_bytes(self.read(8), "big")
        raise ValueError(f"unsupported additional length: {addl}")

    def decode(self):
        if self.pos >= len(self.data):
            raise EOFError("unexpected end of CBOR input")
        initial = self.read(1)[0]
        major = initial >> 5
        addl = initial & 0x1F

        if major == 0:
            return self.decode_uint(addl)
        if major == 1:
            return -1 - self.decode_uint(addl)
        if major == 2:
            length = self.decode_uint(addl)
            return self.read(length)
        if major == 3:
            length = self.decode_uint(addl)
            return self.read(length).decode("utf-8")
        if major == 4:
            items = []
            if addl == 31:
                while True:
                    if self.data[self.pos] == 0xFF:
                        self.pos += 1
                        break
                    items.append(self.decode())
            else:
                length = self.decode_uint(addl)
                for _ in range(length):
                    items.append(self.decode())
            return items
        if major == 5:
            obj = {}
            if addl == 31:
                while True:
                    if self.data[self.pos] == 0xFF:
                        self.pos += 1
                        break
                    key = self.decode()
                    obj[key] = self.decode()
            else:
                length = self.decode_uint(addl)
                for _ in range(length):
                    key = self.decode()
                    obj[key] = self.decode()
            return obj
        if major == 6:
            _ = self.decode_uint(addl)
            return self.decode()
        if major == 7:
            if addl == 20:
                return False
            if addl == 21:
                return True
            if addl == 22 or addl == 23:
                return None
            if addl == 26:
                import struct

                return struct.unpack(">f", self.read(4))[0]
            if addl == 27:
                import struct

                return struct.unpack(">d", self.read(8))[0]
        raise ValueError(f"unsupported CBOR major/additional: {major}/{addl}")


def extract_manifest(pack_path: Path) -> dict:
    with zipfile.ZipFile(pack_path, "r") as zf:
        data = zf.read("manifest.cbor")
    decoder = CBORDecoder(data)
    manifest = decoder.decode()
    if not isinstance(manifest, dict):
        raise ValueError(f"{pack_path} manifest is not a CBOR map")
    return manifest


entries = []
for pack_dir in sorted(packs_dir.iterdir()):
    if not pack_dir.is_dir():
        continue
    pack_name = pack_dir.name
    gtpack_path = pack_dir / "dist" / f"{pack_name}.gtpack"
    if not gtpack_path.exists():
        continue
    manifest = extract_manifest(gtpack_path)
    out_path = manifest_dir / f"{pack_name}.manifest.json"
    out_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    entries.append(f"{pack_name}\t{gtpack_path}\t{out_path}")

packs_list_path.write_text("\n".join(entries) + "\n")
PY

for pack_dir in "${PACKS_DIR}"/*; do
  pack_name="$(basename "${pack_dir}")"
  lock_path="${pack_dir}/pack.lock.json"
  if [ -f "${lock_path}" ]; then
    cp "${lock_path}" "${LOCK_DIR}/${pack_name}.lock.json"
  fi
done

echo "Wrote manifests to ${MANIFEST_DIR} and locks to ${LOCK_DIR}."
