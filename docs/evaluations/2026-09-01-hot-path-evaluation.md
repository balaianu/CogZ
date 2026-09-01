# Hot Path Evaluation — Agent Perspective

Date: 2026-09-01
Method: Ran the actual hot paths (cold start, task mode, escalation, search) as an agent would — read the output, evaluated whether it helps or wastes tokens.
Repo: CogZ dogfooding against itself (1135 entities, 3035 edges, schema v4, both models available)

---

## What works

### Search itself is good

`cogz search "RRF fusion ranking"` returns `rrf.rs`, `hybrid.rs`, the `fuse` function, and the "Search pipeline" knowledge entry — exactly what an agent would need. The dual-model search, graph edges, and FTS all function correctly. Search is the strongest part of the system.

### Auto-linking works

The "Search pipeline" knowledge entry has `auto_references` edges to `hybrid.rs`, `fuse`, `fts_search`, `knn_search`, `SearchConfig` — the graph correctly connects knowledge to code.

### Knowledge content is genuinely useful

The "Search pipeline — FTS, vector, RRF, graph expansion" entry is exactly what an agent would want to read before touching the search code. It explains the 6-step pipeline, the over-fetch strategy, the batched BFS. This is real value.

---

## What doesn't work

### 1. Context pack token allocation is wrong for code tasks

Query: "fix the search pipeline RRF fusion ranking"

Result:
- 6 rules (NLI softmax, DB conventions, mutex reentrancy) — ~60% of the budget
- 4 knowledge entries — the rest
- 0 code entities — all dropped

The actual `fuse()` function, `hybrid.rs`, `SearchConfig` — the things an agent would need to touch — were dropped at position 260 in the dropped list. Rules have priority 1, knowledge priority 2, code priority 3. For a code task, that ordering is backwards.

### 2. Cold start code map is noise

The "Code Map — Modules" section lists "tests" 19 times. 39 out of 122 modules in the DB are `#[cfg(test)] mod tests` blocks — they're not real modules. The "Key Files" section ranks `test_mcp_server.rs` (36 symbols) as the most important file. Test files dominate because they have many small functions. This doesn't tell an agent the project structure — it tells them which test files are longest.

### 3. Access tracking doesn't influence cold start

Access counts only increment on search results. Cold start doesn't use search, so it never increments access for rules. Rules all have `access_count=0`. Knowledge entries have `access_count=12-13` (from search results), but the difference between 12 and 13 in the composite score is 0.008 — negligible. The scoring sorts, but the sort is essentially arbitrary because all entities have similar access counts and similar recency.

### 4. Composite score isn't surfaced

Cold start computes a `cold_start_score` and uses it for sorting, but sets `relevance: 0.0` on every section. The output shows "relevance: recent" for everything. The agent consuming the pack has no idea which rules were scored highest vs. just included by default. The score is computed and thrown away.

### 5. Graph-expanded code entities have relevance 0.0

In task mode, code entities reached via graph expansion get `relevance: 0.0`. They're included but unranked, so they sort to the bottom and get dropped first by the token budget. The entities most relevant to the task (the actual code to edit) are the first to be cut.

### 6. The dropped sources list is noise

260 dropped sources are printed to the agent's context. That's ~260 lines of "X over token budget" that the agent has to read past. It's metadata about what wasn't included, which is less useful than just including more of the right things.

---

## Does it make a difference in the right direction?

Directionally yes, operationally not yet. The infrastructure is right — dual embedding spaces, graph edges, auto-linking, composite scoring, access tracking. But the last mile (token allocation, score surfacing, code map quality) means the agent gets rules and knowledge but not the code they need. For a cognition runtime aimed at coding agents, that's the wrong trade-off.

The system is more useful than no context at all — the rules and knowledge entries are genuinely informative. But for a code task, an agent would be better served by `cogz search` (which finds the right code) than by `cogz context --mode task` (which drops it).

---

## How to measure impact

### 1. Task-relevant retrieval precision

Define 20-30 representative tasks ("fix RRF fusion", "add a new MCP tool", "change embedding model"). For each, identify the ground-truth files/functions an agent would need to touch. Run `cogz context --mode task <query>` and measure: what fraction of ground-truth entities appear in the kept sections (not dropped)?

Current baseline: for "fix RRF fusion", the ground truth is `rrf.rs`, `hybrid.rs`, `fuse()`, `SearchConfig`. Retrieval precision = 0/4.

### 2. Token efficiency

Of the tokens spent, what fraction went to entities the agent actually used vs. ignored? This requires running an agent on a task and tracking which context sections it referenced. Low efficiency = most of the budget spent on rules the agent already knows.

### 3. Noise ratio

Kept sections / total candidate sections. Currently 13/273 = 4.8%. If the kept 4.8% is the right 4.8%, that's fine. If it's 6 rules + 4 knowledge entries and 0 code for a code task, the ratio is misleading — it's "efficient" but wrong.

### 4. Cold start utility decay

Run cold start on a fresh DB (all access_count=0) vs. after 50 searches. Does the pack change? It should, if access scoring works. Right now it doesn't change because cold start doesn't increment access counts and the score difference is negligible.

### 5. A/B task completion

Give an agent the same task with (a) CogZ context pack and (b) equivalent tokens of `rg` output. Which completes the task faster/with fewer errors? This is the ultimate test — does the structured context beat raw search?

---

## Most impactful fixes (ordered)

1. **Task mode: allocate token budget by entity type based on query.** A code query should give code entities 50% of the budget, not 0%. The current priority ordering (rules > knowledge > code > observations) is backwards for code tasks.

2. **Surface the composite score in the relevance field.** Set `relevance` to the actual score for cold start sections and graph-expanded entities, not 0.0. The consuming agent needs to know what's high-confidence vs. filler.

3. **Filter test modules from the code map.** `#[cfg(test)] mod tests` blocks are not structural modules. Exclude them from the module list, or weight them lower.

4. **Rank key files by structural importance, not symbol count.** Use incoming edge count (how many other files reference this file) instead of raw symbol count. Test files have many symbols but low structural importance.

5. **Suppress or truncate the dropped sources list.** Print a summary ("260 sections dropped, 4096 token budget") instead of 260 individual lines.

6. **Increment access counts in cold start and context assembly**, not just search. Otherwise access tracking only reflects search behavior, not overall usage.
