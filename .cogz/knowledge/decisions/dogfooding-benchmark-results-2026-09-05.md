---
id: 43fb22d9-f9aa-48ae-80e8-be4664bc8b36
title: Dogfooding benchmark results — 2026-09-05
type: knowledge
status: stale
created_at: "2026-09-05T07:41:48.634115327+00:00"
updated_at: "2026-09-06T06:35:34.308338721+00:00"
references: ["4dca380d-5f31-5532-bc15-ed61df929434", "66bf462e-c70e-564b-828f-4e354e24cd9b", "de4d1cde-3233-5d67-bb3b-0ab2330c47ae"]
category: decisions
tags: ["benchmark", "dogfooding", "quality", "metrics"]
---

Full end-to-end dogfooding benchmark run on 2026-09-05. 54 queries across all retrieval paths via MCP stdio and CLI.

## Results Summary

| Path | Count | Avg P@5 | Avg MRR | Avg Recall | Cov% | Avg Time |
|---|---|---|---|---|---|---|
| search (hybrid) | 20 | 0.420 | 0.731 | 0.967 | 100% | 1.71s |
| search --code | 10 | 0.420 | 0.800 | 0.900 | 100% | 1.08s |
| context_task | 10 | 0.860 | 0.900 | 0.967 | 100% | 1.45s |
| context_cold_start | 1 | 1.000 | 1.000 | 1.000 | 100% | 0.20s |
| context_escalation | 5 | 0.840 | 1.000 | 1.000 | 100% | 1.60s |
| query_* | 3 | 1.000 | 1.000 | 1.000 | 100% | 0.10s |
| list_entities | 5 | 1.000 | 1.000 | 0.885 | 60% | 0.10s |
| **OVERALL** | **54** | **0.637** | **0.845** | **0.952** | **96%** | **1.27s** |

## What works well

1. **Context assembly is excellent.** Task mode averages P@5=0.86, MRR=0.90. Cold_start and escalation are perfect. The graph expansion + RRF fusion + token budgeting pipeline produces highly relevant packs.
2. **Query/list tools are perfect.** 100% precision, 100% MRR, instant response. These are simple DB lookups but they work flawlessly.
3. **Coverage is 100% for search and context.** Every query returned enough results. The 40-result default is sufficient.
4. **Latency is good.** Search ~1.7s (after model load), context ~1.5s, queries ~0.1s. Cold_start is 0.2s (FTS-only, no model needed).
5. **Consolidation is conservative.** 0 false positives on a clean knowledge base. No unwanted promotions or merges.
6. **Lifecycle events work.** session_start produces context packs, session_end runs consolidation, both record events.

## Weak spots

1. **Search P@5 is low (0.42).** The top-5 results often contain graph-expanded entities (modules, auto-references) that are technically related but don't contain the query keywords. This is the known graph-expansion-dilutes-precision issue.
2. **Two queries had P@5=0:** s05 ("config staleness mtime reload") and s20 ("graceful degradation FTS only"). These are multi-word queries where the relevant knowledge exists but doesn't rank in top-5 due to graph expansion pushing less relevant code entities up.
3. **c02 ("spawn_blocking tokio async") had MRR=0.** The code search didn't find the relevant function in results at all. This may be the code/knowledge embedding mismatch — the query uses the code model but the function's embedding was created with the knowledge model (or vice versa).
4. **list_knowledge and list_observations failed coverage** (19/25 and 10/15). The knowledge base has 19 knowledge entries and 10 observations — fewer than the benchmark expected. This is a benchmark calibration issue, not a CogZ bug.

## Actionable improvements

1. **Graph expansion dilution.** Consider downranking graph-expanded entities vs direct search hits in RRF fusion. Currently graph-expanded entities get the same relevance score as direct hits.
2. **Code embedding mismatch.** The known issue: code entities embedded with CodeRankEmbed but query embedding uses bge-base (knowledge model). Separate KNN queries per model would fix this.
3. **Search could benefit from a relevance threshold.** Currently all 40 results are returned regardless of score. A threshold would filter out low-relevance graph-expanded noise.