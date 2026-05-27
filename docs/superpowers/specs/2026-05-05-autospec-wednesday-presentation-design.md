# Autospec Wednesday Presentation Design

## Goal

Create a twenty-two-slide light-blue executive presentation that opens with a title slide called "Last 2 Weeks Work", then explains why autospec is needed, shows the request-to-issue-tree workflow, explains autospec's evolution, walks through the local-model hardware search caused by cloud quota exhaustion, explains why LLM context length directly affects local memory and speed, introduces ChemLake/Nexus as the chemistry evidence workspace, closes the LCB-adjacent last-two-weeks delivery section with throughput, and ends by noting that autospec also generates documentation and a rough publication draft.

## Audience

Wednesday meeting attendees who need a fast product-level understanding of autospec without reading the repository.

## Visual Style

Use the global PowerPoint guidelines in `~/.codex/powerpoint-style-guidelines.md`: light background, refined blue system, spacious cards, and simple sketch-style diagrams.

The deck should use one cohesive design language across all slides, matching the older `artifacts/presentations/old.key` visual direction:

- Same header hierarchy on every slide: small blue eyebrow, large navy title, readable gray subtitle, and top-right slide number.
- Light-blue background, white or pale-blue surfaces, saturated blue emphasis, and subtle sketch-style connector lines.
- Large airy cards with generous internal padding, soft blue outlines, and small purposeful accent ticks instead of heavy flat blocks.
- A pale-blue takeaway band near the bottom of each slide.
- Purposeful accent colors only where they add meaning: warm red for problem state, cyan/green/amber for secondary lanes.
- Avoid nested dense boxes, tiny pills, duplicated labels, and text that relies on the presenter decoding shorthand.
- Keep slide 1, slides 4-9, and slides 10-12 visually related while preserving the older deck's stronger "wow" factor.

## Narrative

1. **Title.** Last 2 Weeks Work frames the full update for a mixed audience.
2. **Why autospec is needed.** AI can generate large volumes of code quickly, but without specs and issue history teams lose the ability to explain why code exists, what it was meant to do, and how it was generated.
3. **Intent to shipped PRs.** Autospec turns a feature request into a design spec, then a linked 1:n issue tree with epics, tasks, and sub-issues, followed by implementation PRs and merged work.
4. **Product evolution.** Autospec began as Claude Code skills, was refactored into a tool coordinated through the minion issues browser, and collapsed into one unified multi-harness skill.
5. **Quota cliff.** Codex and Claude usage ran out during high-throughput autospec work, forcing a local open-source model fallback path.
6. **Local hardware comparison.** Qwen 3.6 35B-A3B exposed the difference between unified memory and discrete VRAM: a 48GB MacBook could run the experiment while a 22GB-VRAM workstation could not do so comfortably despite 512GB RAM.
7. **Frontier-model bottleneck.** GLM-5.1 can run with CPU/GPU offload, but the observed ~2 tokens/sec is far below the 20-50 tokens/sec target for productive development; a 256GB Mac Studio is the plausible next test but is expensive and hard to obtain.
8. **Context window reality.** Context is the LLM's working memory: Claude hides the memory cost in the provider, while local Qwen 3.6 and GLM-5.1 expose long-context cost as VRAM/cache pressure, offload, latency, and lower tokens/sec.
9. **ChemLake proof point.** The private `metabolomics-us/chemlake` repository shows the pattern in practice: a checked-in ChemSpider spec generated a linked issue tree with an epic, implementation issues, model-fit labels, and PR activity.
10. **Naming decision.** Claude and Codex both agree that ChemLake is a poor public name; ChemLake remains the internal data-lake codename and Nexus becomes the publication/general-use name.
11. **Nexus product story.** ChemLake is the internal lake and source-ingestion engine; Nexus is the public workspace that turns source databases into explainable chemistry evidence.
12. **Nexus architecture.** Hive/Quobyte, Slurm, NATS, and bounded APIs make the internal lake usable without exposing users to raw lake complexity.
13. **Live Hive coverage.** Real normalized Parquet counts from May 5, 2026 show usable coverage today and concrete source gaps for the next buildout.
14. **Aspirin lookup showcase.** A single aspirin query demonstrates deterministic cross-source evidence: identity, synonyms, drug evidence, labels, targets, mappings, and query-performance caveat.
15. **LCB reminder title.** A short section divider makes the final delivery updates read as LCB-relevant operational and scientific guardrails rather than a separate status dump.
16. **go-modules recovery functionality.** Users can move from "sample is stuck" to bounded recovery actions: retry, repair, inspect logs, or accept a deterministic failed state.
17. **go-modules operator/runtime surfaces.** Operators can see queues, files, nodes, logs, and audit state while safer worker boundaries keep automation controlled.
18. **TUI diagnostics.** Sample state and upload events become a traceable timeline for diagnosing lab upload issues.
19. **WCMC chemical identity and STEAC functionality.** Scientists and operators get safer chemical identity handling, inspectable generated-bin decisions, fewer silent promotions, and clearer failure behavior.
20. **Shared delivery pattern.** Across projects, the useful pattern is functionality users can understand and operate: recover stuck work, inspect real state, protect scientific decisions, and preserve why each change happened.
21. **Throughput.** Autospec converted two weeks of work across the discussed repositories into a visible issue graph with 709 created issues, 578 solved issues, and a daily stacked closure plot by project.
22. **Publication draft example.** Autospec also turns the same evidence trail into generated documentation and a rough Nature Methods-style abstract draft, so teams keep a narrative seed alongside code, issues, and PRs.

## Slide Specification

### Slide 1: Last 2 Weeks Work

- Main message: frame the whole update before the audience sees the details.
- Use the old blue presentation style: large title, short subtitle, three symmetric agenda cards, and a pale-blue footer.
- Cards: Autospec delivery, local model reality, and LCB + ChemLake.
- Keep the slide spacious and executive-readable.

### Slide 2: Make AI-generated code explainable

- Main message: the point is not more code; the point is code with memory.
- Reference the go-modules cautionary example: the project grew by close to 100k lines of code without a clear historical record of why changes were made or what was added.
- Show a polished before/after: code volume without a trail versus generated work backed by specs, issues, labels, PRs, and review history.
- Use the shared three-card system: without a trail, autospec bridge, and with memory.

### Slide 3: Autospec turns intent into shipped work

- Main message: one user request can become a structured delivery pipeline.
- Use compact step cards for request -> design spec -> 1:n linked issue tree -> PR loop -> merged output, plus three supporting explanation cards.
- Include supported harnesses: Claude Code, OpenCode, Codex CLI.

### Slide 4: From Claude skills to one autospec skill

- Main message: autospec evolved from prompt collections into a portable operating contract.
- Show the timeline:
  - Original Claude Code skill experiments proved the spec-first workflow.
  - The workflow was refactored into a tool using different specialized agents for research, planning, issue creation, implementation, and review.
  - The minion issues browser coordinated visibility, ownership, queues, issue trees, and PR progress.
  - The system collapsed back into one unified skill shared across Claude Code, Codex CLI, and OpenCode.
- Explain why the collapse matters:
  - One canonical skill avoids prompt drift across harnesses.
  - Issue sizing and model-fit labels keep context controlled.
  - Tiered routing keeps expensive reasoning on spec stages and cheaper loops on implementation.
  - Subagents are used heavily, but only for bounded parallel work that improves the result.
- Use the shared card language for the four-stage timeline and two supporting explanation cards.

### Slide 5: Quota limits forced local model experiments

- Main message: Codex and Claude usage exhaustion turned local open-source inference from an optimization into a necessity.
- Explain the practical problem: autospec, subagents, issue decomposition, review loops, and retries create high token demand.
- Show the target: not just "can the model run," but "can it run at a development-useful speed."
- Include the working threshold used in the presentation: around 20-50 tokens/sec is the target range for coding-agent usefulness; around 2 tokens/sec is not interactive enough.
- Compare two candidate model paths:
  - Qwen 3.6 35B-A3B: small MoE path that fit the MacBook 48GB experiment.
  - GLM-5.1: frontier-class agentic coding target with much larger memory pressure.

### Slide 6: Qwen 3.6 exposed the memory architecture gap

- Main message: unified memory can make a smaller MacBook more useful for local LLM testing than a larger workstation when the workstation has insufficient VRAM.
- Show three comparison cards:
  - MacBook Pro path: 48GB unified memory, roughly $2.4k, successfully runs the 35B-class MoE experiment.
  - Workstation path: 512GB system RAM, 22GB VRAM, blocked by discrete-GPU memory limits.
  - GPU upgrade path: RTX 6000 Ada-class 48GB VRAM upgrade, roughly $6k-$7k, not a sensible near-term purchase.
- Explain the audience-level distinction: system RAM is not equivalent to VRAM for local inference on a discrete GPU.

### Slide 7: GLM-5.1 runs, but not fast enough yet

- Main message: GLM-5.1 can be made to run by offloading most computation into system memory, but the resulting speed is not usable for development.
- Show three cards:
  - Feasible on workstation: hybrid CPU/GPU placement avoids a hard VRAM failure.
  - Not interactive: observed throughput is about 2 tokens/sec.
  - Blocked option: a 256GB unified-memory Mac Studio is the likely next experiment but costs around $5k and is hard to find due to high-memory Mac supply constraints.
- Show a simple speed bar: 2 tok/s current path versus 20-50 tok/s usable target.
- Web research notes:
  - Z.ai documents GLM-5.1 as its latest flagship model for long-horizon and agentic coding workflows.
  - Public GLM-5.1 summaries describe it as a very large open-weight MoE model, so local deployment is primarily a memory-placement problem.
  - 2026 Mac Studio high-memory configurations have been supply constrained, with top RAM options removed or delayed.
  - RTX 6000 Ada-class cards provide 48GB VRAM but sit in the multi-thousand-dollar workstation GPU price tier.

### Slide 8: Context is memory, latency, and accuracy budget

- Main message: context length is not just a model-feature number; it consumes memory, slows inference, and can reduce agent usefulness if it forces offload.
- Explain context for a mixed audience:
  - Context is the model's working memory: prompt, chat history, tool output, file excerpts, and generated state.
  - Longer context means more tokens must be attended to and more KV/cache state must be kept during generation.
  - Cloud models hide local memory pressure; local models expose it directly as VRAM, unified memory, offload, and tokens/sec.
- Compare the three model paths:
  - Claude: provider-managed long context, 200K standard with 1M available in eligible deployments; the user pays quota/cost/latency rather than local VRAM.
  - Qwen 3.6 local MoE: smaller open model path that fit the 48GB unified-memory MacBook experiment.
  - GLM-5.1: 200K frontier-class long-context target, but much larger MoE weights make local offload the dominant bottleneck.
- Include a simple horizontal bar plot estimating local context-cache memory pressure for common context sizes: 16k, 64k, 128k, and 200k tokens.
- Show approximate KV/cache-only values: 1.5GB, 6.0GB, 12.0GB, and 18.8GB, with a 22GB VRAM marker. Label that model weights, runtime overhead, and offloaded layers are additional.
- Audience takeaway: Claude hides memory pressure in the cloud; local Qwen 3.6 and GLM-5.1 experiments expose it directly as VRAM, offload, and tokens/sec.
- Source notes:
  - Anthropic documents Claude context windows as 200K standard, with 1M available for eligible deployments.
  - Z.ai documents GLM-5.1 as a 200K-context long-horizon model.
  - Qwen and GLM Hugging Face configs show the architectural fields used to reason about KV/cache size; the plotted values are an explanatory local-estimate, not a measured profiler result.

### Slide 9: ChemLake proof point

- Main message: ChemLake already uses autospec-style decomposition to convert specs into model-sized issue trees and PRs.
- Include repository data fetched from `metabolomics-us/chemlake` on 2026-05-05:
  - 1 tracked `docs/specs/*.md` file.
  - ChemSpider spec generated 9 linked issues: #565 epic plus #566-#572 and #576.
  - 8 model-fit implementation nodes: 4 `ctx:32k`, 4 `ctx:64k`, all `reasoning:medium`.
  - Repo-wide activity: 354 issues, 224 PRs, 210 merged PRs, 101 PRs tied to issue-closing/auto branches.
  - Model/harness evidence: 166 `llm-ready` issues, 142 `batch:small` issues, 99 merged auto/issue-closing PRs, and 75 `codex/*` PR branches.
- Use shared metric cards and explanation cards showing spec-to-tree, model-fit routing, and PR evidence.

### Slide 10: Naming decision: ChemLake internal, Nexus public

- Main message: ChemLake remains the internal data-lake/repository codename; Nexus is the publication and general-use name.
- State the naming decision plainly: Claude and Codex both think “ChemLake” is a bad public name.
- Show a simple rename-boundary diagram:
  - ChemLake: internal engineering name, Hive data lake, source ingestion, normalized evidence tables.
  - Nexus: public/product name, user workspace, queries, sharing, and publication-facing chemistry evidence.
- Bottom chips: internal scope = ChemLake; public scope = Nexus; publication copy = Nexus; code/repo roots = ChemLake.
- Audience takeaway: ChemLake is what we build and operate internally; Nexus is what users and publications should remember.

### Slide 11: Nexus is a chemistry evidence workspace

- Main message: ChemLake is the internal evidence lake; Nexus is the publication and user-facing workspace.
- Show a clean left-to-right product story:
  - Source databases: PubChem, HMDB, DrugBank, DailyMed, GNPS, MoNA, MassBank, SMPDB, and future lanes.
  - ChemLake internal lake: raw snapshots, normalized identity rows, provenance, work manifests, and release state.
  - Nexus user surface: compound lookup, saved queries, rerun/diff, sharing, and publication-ready evidence.
- Keep the data anchors readable: about 11 TB raw lake, 30+ planned source lanes, and a visible shift from database plumbing to workspace outcomes.
- Bottom takeaway: users should remember Nexus; ChemLake remains the internal engine.

### Slide 12: How the internal lake becomes a usable workspace

- Main message: Hive and Slurm do the heavy lifting inside ChemLake; NATS and bounded APIs turn that work into user-facing Nexus queries.
- Show the architecture as a simple flow:
  - Hive/Quobyte: raw snapshots, work manifests, logs, releases, and normalized Parquet close to the data.
  - Slurm execution: orchestrators recover stale claims, workers process source jobs, and jobs publish releases.
  - NATS events: worker lifecycle, job progress, cancellation, and request/reply boundaries.
  - Nexus workspace: saved queries, rerun/diff, sharing, and ChemLake-aware model routing.
  - Bounded API surface: translation and predefined service queries without exposing raw lake internals.
- Include a compact metric strip: 11 TB raw lake, 3.9 GB normalized, 329 ChemLake closed, 63 Nexus closed, 76 PubChem closed, and 54 open work.
- Keep the slide architectural, not a backlog dump.

### Slide 13: Live ChemLake Hive content counts

- Main message: ChemLake now has real normalized chemical identity coverage on Hive, not just planned source lanes.
- Use live read-only Hive normalized Parquet metadata gathered on May 5, 2026.
- Show the counts as insight groups rather than a dense table:
  - PubChem: 247,694,674 identity rows in `snapshot=2026-05-05-overnight-poc`.
  - DrugBank: 19,858 distinct compound IDs from 3,963,135 identity rows.
  - HMDB: 217,920 distinct compounds.
  - MoNA: 851,895 identity rows.
  - GNPS: 600,328 identity rows.
  - MassBank: 22,311 identity rows.
  - DailyMed: 8,071 distinct compounds in full-release labels, plus 68 in daily snapshot.
  - SMPDB: 53,239 identity rows.
  - Blood Exposome: 67,150 identity rows.
- Show missing/discovered compound identity lanes: ChEBI, ChEMBL, KEGG, CAS Common Chemistry, ECHA, FooDB, T3DB, LipidMaps, PubMed, Reactome, Wikidata, MeSH, clinicaltrials, CompTox/DSSTox, and related registered lanes.
- Do not call PubChem identity rows “distinct compounds” unless the distinct query completes.

### Slide 14: Aspirin lookup showcase

- Main message: one compound lookup becomes an auditable cross-source evidence graph.
- Use two large panels rather than a dense table:
  - Resolved identity: formula, InChIKey, SMILES, and compact identifier tags.
  - Source contributions: PubChem, DrugBank, HMDB, DailyMed, targets, and cross-references.
- Show aspirin / acetylsalicylic acid identity:
  - Formula `C9H8O4`.
  - InChIKey `BSYNRYMUTXBXSQ-UHFFFAOYSA-N`.
  - SMILES `CC(=O)OC1=CC=CC=C1C(O)=O`.
  - DrugBank `DB00945`, PubChem CID `2244`, HMDB `HMDB0001879`, CAS `50-78-2`, ChEBI `15365`, ChEMBL `CHEMBL25`, RxCUI `1191`, UNII `R16CO5Y76E`, KEGG `C01405` / `D00109`.
- Show deterministic local evidence categories:
  - Synonyms: ASA, acetylsalicylic acid, O-acetylsalicylic acid, Aspirina, Bufferin, Ecotrin, Durlaza, Polopiryna, and multilingual/brand variants.
  - DrugBank: approved drug indication, proxy strength `0.65`, approved/investigational/veterinary-approved groups, and target evidence including COX-1 and COX-2 inhibitor records.
  - DailyMed: aspirin product labels including low-dose, Bayer, enteric-coated, delayed-release, and combination products.
  - PubChem: CID `2244` substance mappings including Tox21, Debye Scientific, and Innovapharm SIDs.
- Add caveat: this query took about 20 minutes because the backend still uses the large remote databases inefficiently; research is ongoing to make Hive-scale queries remotely usable.
- Note that no separate exact “asperin” entity was found; useful records resolve under aspirin / acetylsalicylic acid.

### Slide 15: LCB delivery section

- Main message: the final section is not a separate status dump; it shows how autospec-assisted changes landed LCB-relevant operational and scientific guardrails.
- Use a short section-divider slide before the moved last-two-weeks updates.
- Show three cards: go-modules, WCMC, and LCB relevance.
- Explain that the final section contains concrete operational and scientific workflow progress from the last two weeks: fewer stuck jobs, more visible production state, and safer identity automation.

### Slide 16: go-modules recovery and queue control became actionable

- Main message: users can move from stuck samples and queue pressure to bounded recovery paths, controlled retries, and clearer failure causes.
- Show three symmetric explanation cards: recover failed work, control queue pressure, and explain failure causes.
- Keep the card grid sparse enough for a mixed audience: one explanatory paragraph plus one result callout per card.
- Audience takeaway: production failures became recoverable states with controlled retries and clearer operator decisions.
- Include examples: failed-conversion recovery (#191-#198), missing-result recovery (#229), zombie message repair (#234), backlog relief (#224), transient retry routing (#369), duplicate aggregation guard (#226), explicit aggregation failure reasons (#225), JAR failure state (#237), and segfault recovery (#200/#228).

### Slide 17: go-modules operator surfaces and runtime safety converged

- Main message: production state became visible and automated execution boundaries became safer.
- Show three symmetric cards: operate from GUI/TUI, audit jobs and files, and run workers safely.
- Condense the earlier operator-surface and runtime-boundary material into this single slide to reduce end-section density.
- Audience takeaway: diagnosis and execution moved from ad hoc operations into purpose-built, bounded workflows.
- Include examples: GUI primitives/pages/live push (#209-#223/#227), job-file audit CLI and docs (#273-#278), audit navigation/native SSH (#310-#317), input hints/footer scoping (#292-#302), canonical Slurm paths (#390), LLM log lookup (#416/#418/#420), and live Slurm node health (#394/#395).

### Slide 18: TUI sample state and upload event tracking

- Main message: add a screenshot-ready slide for the TUI that traces sample state and upload events.
- Use a large terminal-style screenshot frame on the left, with a placeholder that can be replaced by the real TUI screenshot.
- The screenshot should show sample ID, state, event, timestamp/location, and enough context to diagnose lab upload issues.
- Show three explanation cards: diagnose upload timing, trace location, and find lab issues.
- Audience takeaway: state and event tracking turns lab upload problems into a traceable timeline instead of a late-stage mystery.

### Slide 19: WCMC chemical identity decisions got guardrails

- Main message: STEAC and generated-bin work focused on preventing bad automatic associations and making identity decisions easier to inspect after the fact.
- Show three larger explanation cards:
  - Protect identity decisions: assay-version resets, scan-correlation gates, and promotion diagnostics reduce silent promotions of weak identities.
  - Inspect generated bins: association metadata and profile integration coverage help reviewers trace why an identity exists.
  - Surface STEAC failures: persistence failures rethrow, fast identity grouping has a concrete direction, and build blockers no longer hide validation.
- Audience takeaway: chemical identity automation is becoming explainable enough to review and constrained enough to trust.
- Include examples: assay-version reset (#717), merge gate (#718), promotion diagnostics (#729), generated-bin metadata (#719), STEAC profile integration coverage (#720), local test preservation (#723), persistence failure surfacing (#740/#741), fast identity grouping spec (#746), and build hygiene (#693).

### Slide 20: One delivery pattern across multiple systems

- Main message: the useful pattern is not PR count; it is production problems becoming scoped, reviewed, and delivered as functionality users can operate.
- Show three user outcomes with short explanations:
  - Recover stuck work: failures become states with bounded retry, repair, and inspection paths.
  - Operate real state: queues, files, nodes, logs, and samples are visible through production tools.
  - Protect decisions: scientific identity handling gets tests, diagnostics, and gates before automation promotes results.
- Show four autospec principles: specify intended behavior, slice work into bounded changes, verify behavior with checks, and preserve the rationale.


### Slide 21: Autospec issue throughput

- Main message: autospec solved 578 issues in the last two weeks and made the closure pattern visible by project.
- Use GitHub search counts from 2026-04-22 through 2026-05-06 across `berlinguyinca/autospec`, `metabolomics-us/go-modules`, `metabolomics-us/wcmc`, and `metabolomics-us/chemlake`.
- Show three large number cards:
  - 709 issues created: autospec 110, go-modules 140, WCMC 47, ChemLake 412.
  - 578 issues solved/closed: autospec 109, go-modules 83, WCMC 14, ChemLake 372.
  - 73 workflow-generated issues manually generated by the autospec workflow in the same window.
- Add a stacked bar plot of closed issues per day, stacked by project:
  - ChemLake: 372 closed.
  - autospec: 109 closed.
  - go-modules: 83 closed.
  - WCMC: 14 closed.
- Mark the data as refreshed on May 6, 2026 after the overnight run.
- Keep this as the final delivery-throughput slide so the audience sees concrete progress before the documentation/publication closer.

### Slide 22: Autospec can draft the Nature Methods abstract too

- Main message: autospec does not stop at issues and PRs; it can also use the ChemLake/Nexus documentation trail to draft publication prose.
- Use the Nature Methods Article/Resource abstract limit of up to 150 unreferenced words; the draft artifact is 141 words.
- Show the generated abstract in a large readable card.
- Show four supporting evidence cards:
  - Problem: per-compound web searches and heterogeneous identifiers do not scale reproducibly.
  - ChemLake method: immutable raw snapshots, normalized identities/xrefs/evidence, provenance, DuckDB/Parquet, Hive/Quobyte, Slurm, and NATS-connected workers.
  - Nexus surface: compound lookups, saved queries, reports, audit trails, and MAP/LCB review flow.
  - Current scale: 11 TB raw Hive lake plus normalized PubChem, HMDB, DrugBank, DailyMed, MassBank, GNPS, MoNA, and related evidence.
- Save the complete editable abstract at `artifacts/publications/nexus-nature-methods-abstract.md`.
- Add a caveat band: the abstract is a human-edit starting point and final claims still need validation before submission.
- Audience takeaway: autospec preserves enough context to generate docs and a credible publication-draft seed.

## Acceptance Criteria

- Deck has exactly twenty-two slides.
- Deck uses a consistent light-blue visual style.
- Each slide contains a sketch-style visual.
- Slide 1 is a polished opener for the last-two-weeks update with a clear agenda.
- Slide 2 shows a linked 1:n issue tree rather than a single issue.
- The deck explains that issues are sized and written for different LLM quality/context tiers.
- The deck highlights support for multiple models and harnesses.
- Slide 3 explains the evolution from Claude skills to tool-and-agent orchestration to one unified multi-harness skill.
- Slides 5-7 explain the local-model fallback story, including Codex/Claude quota exhaustion, Qwen 3.6 versus GLM-5.1, hardware constraints, cost tradeoffs, and the current Mac Studio blocker.
- Slide 8 explains what LLM context is and shows a horizontal bar plot of estimated context-cache memory pressure at 16k, 64k, 128k, and 200k tokens.
- Slide 9 includes ChemLake repository counts with the date of retrieval.
- Slide 10 explains that ChemLake is now the internal name and Nexus is the publication/general-use name.
- Slide 11 explains Nexus as the public-facing chemistry evidence workspace powered by internal ChemLake source lanes.
- Slide 12 shows how Hive/Quobyte, Slurm, NATS, and bounded APIs make the internal lake usable from Nexus.
- Slide 13 shows live ChemLake Hive normalized Parquet content counts from May 5, 2026 without overstating PubChem rows as distinct compounds.
- Slide 14 shows the deterministic aspirin lookup as readable source-contribution cards and includes the 20-minute query-performance caveat.
- Slide 15 is an LCB delivery-section transition before the final update slides.
- Slides 16-17 explain condensed go-modules user-facing functionality delivered from 2026-04-22 to 2026-05-06.
- Slide 18 provides a screenshot-ready TUI diagnostics slide for sample state and upload event tracking.
- Slide 19 explains WCMC chemical-identity functionality delivered from 2026-04-22 to 2026-05-06.
- Slide 20 ties go-modules, WCMC, and ChemLake into one repeated autospec-assisted delivery pattern.
- Slide 21 shows the refreshed two-week GitHub issue throughput summary, including `berlinguyinca/autospec`, and includes a stacked bar plot of solved issues by project.
- Slide 22 closes by showing a 141-word Nature Methods-style abstract draft generated from ChemLake/Nexus documentation.
- A reusable global PowerPoint style guide exists at `~/.codex/powerpoint-style-guidelines.md`.

## Generated Issue Slices

- `PRES-EPIC`: Coordinate the presentation narrative and acceptance criteria.
- `PRES-1`: Add the opening Last 2 Weeks Work title slide.
- `PRES-2`: Explain why autospec is needed for documented AI-generated code.
- `PRES-3`: Create opening narrative and 1:n issue-tree pipeline sketch.
- `PRES-4`: Explain autospec's evolution from Claude skills to unified multi-harness skill.
- `PRES-5`: Explain why Codex/Claude quota exhaustion forced local open-source model experiments.
- `PRES-6`: Compare Qwen 3.6 local inference across MacBook unified memory, workstation VRAM, and RTX 6000 Ada-class GPU upgrade paths.
- `PRES-7`: Explain the GLM-5.1 offload experiment, speed bottleneck, Mac Studio 256GB blocker, and local frontier-model next step.
- `PRES-8`: Explain LLM context windows and show estimated local KV/cache memory pressure.
- `PRES-9`: Add ChemLake proof point using live repository counts.
- `PRES-10`: Explain the ChemLake-internal / Nexus-public naming decision.
- `PRES-11`: Explain Nexus as the public chemistry evidence workspace backed by internal ChemLake.
- `PRES-12`: Explain how Hive/Quobyte, Slurm, NATS, and bounded APIs make the internal lake usable.
- `PRES-13`: Add live ChemLake Hive normalized Parquet content counts.
- `PRES-14`: Add deterministic aspirin lookup with query-performance caveat.
- `PRES-15`: Add the LCB reminder title slide before the final delivery update section.
- `PRES-16`: Summarize go-modules recovery and queue-control work from the last two weeks.
- `PRES-17`: Summarize go-modules operator-surface and runtime-boundary work from the last two weeks.
- `PRES-18`: Add screenshot-ready TUI diagnostics slide for sample state and upload event tracking.
- `PRES-19`: Summarize WCMC STEAC and chemical-identity safety work from the last two weeks.
- `PRES-20`: Explain the shared autospec-assisted delivery pattern across go-modules, WCMC, and ChemLake.
- `PRES-21`: Close with two-week issue throughput across the discussed projects.
- `PRES-22`: Add the final Nature Methods-style abstract draft slide and markdown artifact.
