import re

HEADING_RE = re.compile(r"^#{1,6}\s+")
TITLE_RE = re.compile(r"^(#{1,6})\s+(.+?)\s*$")


def heading_title(line):
    match = TITLE_RE.match(line)
    return match.group(2).strip() if match else ""


def append_section(sections, heading, lines):
    sections.append((heading, "\n".join(lines).strip() + "\n"))


def markdown_sections(text):
    sections, current, current_heading, in_fence = [], [], "Document", False
    for line in text.splitlines():
        fence = line.startswith("```")
        in_fence = (not in_fence) if fence else in_fence
        starts_heading = (not in_fence) and bool(HEADING_RE.match(line))
        if starts_heading and current:
            append_section(sections, current_heading, current)
            current = []
        current_heading = heading_title(line) if starts_heading else current_heading
        current.append(line)
    if current:
        append_section(sections, current_heading, current)
    return sections
