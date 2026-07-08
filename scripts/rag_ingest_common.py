import hashlib
import json
from pathlib import Path


def index_canonical(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def write_index_json(path, obj):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as fh:
        json.dump(obj, fh, indent=2, sort_keys=True)
        fh.write("\n")


def text_sha256(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def repo_rel(path, root):
    return Path(path).resolve().relative_to(Path(root).resolve()).as_posix()
