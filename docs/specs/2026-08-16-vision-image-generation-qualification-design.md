# Vision and image-generation qualification — design

**Date:** 2026-08-16
**Status:** Implementation specification (amendment)
**Repo:** berlinguyinca/autospec

**Amends:** [`2026-08-16-multi-model-engineering-team-design.md`](2026-08-16-multi-model-engineering-team-design.md)
— specifically §7 (model capability advertisement / `modalities`), §9 (capability
classes), §21 (UI/UX verification), §32 (calibration) and Wave 11.

**Depends on:** [`2026-08-16-benchmark-per-evaluation-telemetry-design.md`](2026-08-16-benchmark-per-evaluation-telemetry-design.md)
for the `autospec bench` harness and the telemetry contract this amendment extends
with image-specific metrics.

## Benchmark authority

`2026-08-16-repository-derived-real-work-benchmark-design.md` is authoritative for
the benchmark subsystem. This document contributes a **task family** alongside the
RealWork families: the vision and image-generation suites, their accuracy metrics,
and the initial candidate matrix. It does not define a separate harness, CLI, or
result store — `autospec bench` (§49) and the shared result ledger (§53) are the
RealWork spec's, and the `autospec bench vision|image|ui-review` verbs named here
are extensions of that surface.

## Scope binding

The parent spec requires that UI/UX verification dispatch to a model with
`modalities.vision: true` and `automation.browser: true`, and forbids routing
visual validation to text-only models. It does not say how a model *earns* that
vision claim. This amendment supplies the qualification path: benchmark suites,
accuracy metrics and registry fields that move a visual capability from
*advertised* to *calibrated* (parent §8).

Constraints inherited from the parent spec, applying throughout:

- **Advertised is untrusted.** A model claiming vision is not eligible for a
  visual gate until calibration passes (parent §51: model-generated capability
  advertisements are untrusted input until calibrated).
- **Independence holds for visual work.** The separation-of-duties rules in
  parent §4 apply unchanged to generation and visual review — a generator may
  not be the sole judge of its own output, and the model that identified a
  visual defect may not be the sole verifier of the fix.
- **Capability, not modality, is the router key.** `vision_understanding`,
  `ui_visual_review` and `image_generation` are distinct capabilities; accepting
  images does not qualify a model for any of them.

Named models, quantizations and hardware in this document are the initial
candidate matrix, not a fixed routing table — parent §27 (observed beats
advertised) governs which of them the router actually prefers.

## Objective

Extend the AutoSpec model qualification framework with two additional capability classes:

1. Vision / visual understanding
2. Image and graphic generation / editing

These capabilities must be independently benchmarked, versioned, qualified, and exposed to the AutoSpec router.

A model must not be considered generally “multimodal” merely because it accepts images.

---

# Capability Classes

AutoSpec must distinguish at least:

```text
text_generation
code_generation
vision_understanding
ui_visual_review
ocr_document_understanding
diagram_understanding
image_generation
image_editing
graphic_generation
visual_asset_generation
```

Models may advertise one or more capabilities.

---

# Initial Vision Candidate Matrix

## RTX 4090 24 GB

Primary candidate:

```text
Qwen3-VL-8B-Instruct FP8
```

Secondary candidates:

```text
Qwen3-VL-8B GGUF high-quality quant
Gemma 3 27B multimodal, highest practical quant
```

The 8B Qwen3-VL model should be favored initially for interactive UI inspection because it leaves substantial VRAM headroom.

---

## M4 48 GB

Primary candidates:

```text
Qwen3-VL-8B high-quality GGUF
Gemma 3 27B multimodal high-quality quant
```

The Mac should test the highest practical quantization that fits without persistent swap.

---

# Vision Benchmark Categories

## 1. Screenshot Understanding

Create a frozen suite of application screenshots.

Tasks must include:

- identify visible controls
- identify selected states
- find disabled controls
- locate error messages
- identify missing components
- distinguish modal/dialog/background state
- determine whether requested UI exists
- describe visible page hierarchy

---

## 2. UI/UX Review

Provide screenshots containing deliberate problems.

Examples:

- overlapping controls
- clipped text
- poor alignment
- inconsistent spacing
- incorrect responsive layout
- inaccessible contrast
- tiny touch targets
- missing labels
- inconsistent navigation
- confusing hierarchy

The model must identify:

```text
issue
location
severity
reason
suggested correction
```

---

## 3. Visual Regression Detection

Provide:

```text
reference screenshot
candidate screenshot
```

Ask the model to detect meaningful differences.

Benchmark:

- missing elements
- moved elements
- incorrect text
- changed dimensions
- incorrect styling
- broken responsive layouts
- unexpected dialogs
- blank/error states

This capability should eventually integrate with browser automation.

---

## 4. OCR and Text Understanding

Benchmark extraction from:

- screenshots
- terminal screenshots
- forms
- tables
- diagrams
- charts
- documentation images

Measure exact text accuracy separately from semantic understanding.

---

## 5. Diagram Understanding

Provide:

- architecture diagrams
- UML-like diagrams
- sequence diagrams
- data-flow diagrams
- network diagrams
- flowcharts

Ask questions requiring structural understanding rather than simple OCR.

---

## 6. Code + Screenshot Reasoning

Create AutoSpec-specific tasks where the model receives:

```text
source code
UI screenshot
task/request
```

and must determine likely cause of a visual defect.

This is a critical AutoSpec use case.

---

# Vision Performance Telemetry

Every vision evaluation must capture:

```yaml
images:
  count:
  total_pixels:
  input_width:
  input_height:

tokens:
  input:
  output:

performance:
  preprocessing_seconds:
  prompt_processing_seconds:
  generation_seconds:
  total_seconds:
  generation_tokens_per_second:
  time_to_first_token_ms:
```

Also record:

```text
peak VRAM
peak system/unified RAM
power
```

---

# Vision Accuracy Metrics

Depending on task, support:

```text
exact-match accuracy
structured-field accuracy
issue-detection precision
issue-detection recall
F1
severity accuracy
human preference score
```

UI-review benchmarks must penalize hallucinated defects.

A model listing twenty nonexistent UI problems should not outperform a conservative, accurate reviewer.

---

# Initial Image Generation Candidates

## Fast Production Generator

```text
FLUX.2 [klein] 4B
```

Evaluate:

```text
BF16 where practical
FP8
```

Target hardware:

```text
RTX 4090
```

This should be the initial fast generation/editing candidate.

---

## Higher-Quality Graphic/Text Candidate

Evaluate:

```text
Qwen-Image-2512
```

and/or the best current locally deployable Qwen-Image generation/editing model.

Primary interest:

- rendered text
- diagrams
- labels
- UI mockups
- infographics
- presentation assets
- image editing

---

# Optional Additional Candidate

Evaluate:

```text
FLUX.2 [klein] 9B
```

if it fits comfortably and materially improves quality over the 4B variant.

Do not add models merely to increase candidate count.

---

# Image Generation Benchmark Categories

## 1. General Prompt Adherence

Prompts should specify:

```text
subject
composition
style
lighting
number of objects
position
background
aspect ratio
```

Evaluate whether instructions are actually followed.

---

## 2. Text Rendering

This is critical for AutoSpec graphics.

Generate images containing:

- single labels
- multiple labels
- headings
- small paragraphs
- numbers
- filenames
- interface labels
- diagram node text

Measure:

```text
spelling accuracy
layout accuracy
text completeness
readability
```

---

## 3. Diagram Generation

Request:

- system architecture diagram
- flowchart
- deployment diagram
- component relationship graphic
- roadmap graphic
- process diagram

Evaluate:

```text
semantic correctness
label correctness
relationship correctness
legibility
composition
```

---

## 4. UI Mockup Generation

Prompt models to generate:

- dashboard
- settings page
- Kanban board
- mobile interface
- desktop application
- modal
- navigation layout

Evaluate adherence to detailed UI requirements.

---

## 5. Editing

Provide an image plus an edit instruction.

Examples:

```text
remove object
replace text
change label
move object
add UI element
change background
preserve everything else
```

Measure both:

```text
requested-change accuracy
unwanted-change rate
```

Preservation of unaffected content is a first-class metric.

---

## 6. Multi-Reference Editing

Where supported, evaluate:

```text
reference A
reference B
reference C
instruction
```

Measure whether the output correctly combines requested properties.

---

# AutoSpec-Specific Graphic Tasks

Create frozen tasks such as:

### Architecture Graphic

Given a textual architecture specification:

```text
generate a clean architecture overview image
```

Required elements are explicitly enumerated and automatically/human scored.

### Release Graphic

Generate a project/release announcement image using supplied:

```text
project name
version
headline
logo
feature list
```

### Documentation Diagram

Generate a diagram explaining an actual AutoSpec subsystem.

### UI Concept

Generate a proposed UI from an AutoSpec feature specification.

---

# Image Generation Performance Metrics

Image generation does not use tokens/sec as its primary speed metric.

Report:

```yaml
generation:
  width:
  height:
  megapixels:
  steps:
  images_generated:

performance:
  seconds_per_image:
  images_per_minute:
  seconds_per_megapixel:
  peak_vram_mb:
  peak_ram_mb:
  average_power_w:
```

Where the runtime exposes text-encoder throughput, retain it as secondary telemetry.

---

# Quality Metrics for Generated Images

Do not rely on a single automated aesthetic metric.

Combine:

```text
prompt adherence
text correctness
structural correctness
visual quality
editing preservation
human preference
```

Use automated vision-model judging only as one signal.

---

# Cross-Model Judging

Do not allow an image generator to judge its own output as the sole evaluator.

Where possible:

```text
generator != primary vision judge
```

For important qualification tests, combine:

```text
automatic deterministic checks
independent vision model
human/reference evaluation
```

---

# Separation of Duties

Apply AutoSpec separation-of-duties rules to visual work.

Example:

```text
FLUX generates UI mockup
Qwen3-VL reviews it
```

or:

```text
Qwen3-VL identifies UI defect
coding model implements fix
different vision evaluation verifies screenshot
```

A single model should not generate, implement, and independently certify the same visual artifact when an independent evaluator is available.

---

# UI Automation Integration

The vision framework must support future integration with browser automation.

Conceptual workflow:

```text
launch application
      ↓
navigate UI
      ↓
capture screenshot
      ↓
vision model analyzes screenshot
      ↓
compare with acceptance criteria
      ↓
coding agent fixes problem
      ↓
relaunch/re-render
      ↓
capture new screenshot
      ↓
independent visual verification
```

AutoSpec should retain screenshots as benchmark/test artifacts.

---

# Model Registry Extensions

Example:

```yaml
qwen3-vl-8b-4090:
  qualified: true

  capabilities:
    vision_understanding: 0.95
    ui_visual_review: 0.93
    diagram_understanding: 0.91

  performance:
    generation_tps: 84
    screenshot_seconds: 2.8

flux2-klein-4b-4090:
  qualified: true

  capabilities:
    image_generation: 0.92
    image_editing: 0.90
    graphic_generation: 0.86

  performance:
    seconds_per_image: 1.2
```

All values above are illustrative.

---

# Router Integration

The router should choose models by required capability.

Example:

```text
task: inspect broken UI
→ vision model

task: implement UI fix
→ coding model

task: verify corrected screenshot
→ independent vision model

task: generate architecture graphic
→ image-generation model

task: verify labels in generated diagram
→ vision model
```

---

# Benchmark CLI Extensions

Support commands such as:

```text
autospec bench vision <candidate>
autospec bench image <candidate>
autospec bench ui-review <candidate>

autospec bench compare \
  qwen3-vl-8b \
  gemma3-27b-vision

autospec bench compare-images \
  flux2-klein-4b \
  qwen-image-2512
```

---

# Initial Hardware Qualification Matrix

## Linux RTX 4090

Qualify:

```text
Qwen3.8-27B optimized coding worker
Qwen3-VL-8B FP8
FLUX.2 klein 4B
Qwen-Image-2512 where memory/runtime permits
```

Profiles need not remain resident simultaneously.

Implement controlled model switching.

---

## M4 48 GB

Qualify:

```text
Qwen3.8-27B Q8
Qwen3-VL-8B high-quality quant
Gemma 3 27B multimodal
```

Evaluate locally supported image-generation runtimes separately.

Do not assume the Mac will outperform the 4090 for diffusion/image generation.

---

# Acceptance Criteria

This extension is complete when:

- at least one vision model is operational on Linux
- at least one vision model is operational on macOS
- UI screenshot understanding benchmark exists
- visual regression tests exist
- vision inference reports tokens/sec
- FLUX.2 klein 4B is benchmarked on the 4090
- at least one higher-quality image-generation candidate is benchmarked
- text-rendering quality is explicitly measured
- image editing is tested
- image-generation time and memory are recorded
- model registry supports visual capabilities
- router can request vision or generation capabilities
- generation and independent visual review can use different models
- visual artifacts and screenshots are retained with benchmark results