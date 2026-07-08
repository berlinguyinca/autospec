import re


def flush_block(blocks, current):
    blocks.append("\n".join(current).strip() + "\n")
    return []


def section_blocks(section_text):
    blocks, current, in_fence = [], [], False
    for line in section_text.splitlines():
        fence = line.startswith("```")
        in_fence = (not in_fence) if fence else in_fence
        current.append(line) if (in_fence or fence or line.strip()) else None
        current = flush_block(blocks, current) if (fence and not in_fence) else current
        current = flush_block(blocks, current) if ((not in_fence) and (not fence) and (not line.strip()) and current) else current
    if current:
        flush_block(blocks, current)
    return blocks


def split_section(section_text, max_chars, overlap):
    blocks = section_blocks(section_text)
    if not blocks:
        return []
    heading = blocks[0] if re.match(r"^#{1,6}\s+", blocks[0]) else ""
    body = blocks[1:] if heading else blocks
    if not body:
        return [section_text]
    chunks, current = [], heading
    for block in body:
        too_large = len(current + block) > max_chars and current.strip() != heading.strip()
        if too_large:
            chunks.append(current)
            tail = current[-overlap:].lstrip() if overlap > 0 else ""
            current = heading + ((tail + "\n") if tail and tail not in heading else "")
        current += block
    return chunks + ([current] if current.strip() else [])
