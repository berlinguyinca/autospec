from pathlib import Path

from rag_ingest_common import repo_rel


def candidate_paths(root, includes):
    paths = []
    for pattern in includes:
        paths.extend(root.glob(pattern) if any(ch in pattern for ch in "*?[") else [root / pattern])
    return paths


def include_corpus_path(path, rel, seen, excludes):
    if not path.is_file() or rel in seen:
        return False
    return not any(Path(rel).match(pattern) for pattern in excludes)


def corpus_files(root, config):
    corpus = config.get("corpus", {})
    includes = corpus.get("include", ["llms.txt", "llms-full.txt", "docs/**/*.md"])
    excludes = corpus.get("exclude", [])
    seen, files = set(), []
    for path in sorted(candidate_paths(root, includes), key=lambda item: repo_rel(item, root) if item.exists() else str(item)):
        rel = repo_rel(path, root) if path.exists() else str(path)
        if include_corpus_path(path, rel, seen, excludes):
            seen.add(rel)
            files.append(path)
    return files
