# CogZ — Project Goal

## Statement

CogZ is a local-first, code-aware engineering cognition runtime. It
provides persistent memory and contextual retrieval for AI coding
agents working on software repositories.

## What it does

CogZ gives a coding agent three things:

1. **Memory** — it stores observations, rules, and knowledge about a
   codebase, structured by taxonomy and linked to the code itself.
   Memory persists across sessions. An agent that learns something
   about a repository on Monday still knows it on Friday.

2. **Context** — when an agent starts a task, CogZ assembles a context
   pack: relevant observations, rules, knowledge, and the code
   structures they reference. The pack is scoped, ranked, and
   traceable — the agent knows why each piece was included.

3. **Cognition** — CogZ continuously consolidates its memory:
   deduplicates entries, detects contradictions, promotes supported
   observations to rules, and flags knowledge that may be stale when
   code changes. This is not a batch job; it happens on every insert.

## What it doesn't do

- **Project management** — no tasks, plans, sprints, or services. The
  agent already has its own task management. CogZ is memory, not a
  project tracker.

- **Scientific method tracking** — no predictions, outcomes,
  experiments, or validation runs. These are agent concerns, not
  memory concerns.

- **Model hosting** — CogZ uses embedding/NLI models as a consumer,
  not a host. It does not train, fine-tune, or serve language models.
  It calls model traits that wrap ONNX inference or external APIs.

- **Code execution** — CogZ does not run code, build projects, or
  manage toolchains. It indexes and reasons about code; it does not
  execute it.

- **Cloud dependency** — no remote server, no API calls for core
  functionality, no telemetry, no accounts. Everything runs on the
  local machine. The only network access is optional model downloads.

## Who it's for

A developer using an AI coding agent (Devin, Claude, Cursor, etc.) who
wants that agent to accumulate and reuse knowledge about their
codebase across sessions, without relying on cloud-based memory or
context services.

## What success looks like

An agent equipped with CogZ can:

- Recall decisions, bugs, and patterns from previous sessions without
  being told again
- Receive context at session start that orients it to the current
  state of the repository
- Know which observations and rules are about which functions, classes,
  and files — and retrieve them together
- Detect when its knowledge might be stale because the referenced code
  has changed
- Do all of this on a resource-constrained machine (7 GB RAM, 2012-era
  CPU) alongside other running services

## Relationship to CogZ-py

CogZ-py (the Python version) was the prototype. It validated the core
concepts — hybrid search, taxonomy scoping, knowledge files,
MCP-first interface — and revealed the structural problems that this
rewrite addresses: scope creep, dual storage, monolithic files, model
coupling, and superficial code-awareness.

The Python codebase is archived, not deleted. It serves as reference
for what worked and warning for what didn't.
