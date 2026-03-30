The evolution of the Cortex daemon into a collaborative institutional brain requires an architecture that can handle high-frequency concurrent ingestion while maintaining strict provenance and data isolation. In a team environment where multiple users (e.g., Alex, Mark) and their respective AI agents contribute to the same persistent store, memory is no longer just a "log"—it becomes an **Evidence Ledger**. This report synthesizes 40+ sources to define the implementation of tiered access control, multi-tenant concurrency, and provenance-linked retrieval for the Cortex daemon.

## **1\. The Tiered Memory Hierarchy: Balancing Privacy and Collective Intelligence**

A collaborative memory system must distinguish between private, ephemeral thoughts and verified project knowledge. Research into the "Memory Fabric" and "Collaborative Memory" frameworks suggests a three-tier representational structure:

* **Private Tier (Working Memory)**: Observations and intermediate reasoning steps visible only to the originating user and their agents. This prevents the "noisy context" problem where one user's trial-and-error distracts the entire team.  
* **Project/Team Tier (Shared Memory)**: Salient decisions, preferred libraries, and architectural rules promoted to the group. Promotion is governed by **Write Policies**, such as requiring manual human "pinning" or consensus from two independent agents.  
* **Global Tier (Canonical Memory)**: Immutable project-wide invariants (e.g., "Deployment target is AWS us-east-1") derived from the repository's .cortex/ configuration.

| Tier | Visibility | Persistence | Consistency Mechanism |
| :---- | :---- | :---- | :---- |
| **Private** | Originating User | Session/User-scoped | Local write |
| **Shared** | Entire Team | Project-scoped | Semantic Rebase 1 |
| **Global** | All Users | Repository-scoped | Git-backed/Immutable |

## **2\. Attribution UI and the Evidence Ledger**

To build trust in a collaborative brain, every retrieved fact must be accompanied by its **Provenance Metadata**. Cortex should implement the **W3C PROV-DM** conceptual model, which defines relationships between entities (facts), activities (tool uses), and agents (Alex, Mark, or specific AI models).

### **Attribution Pattern: \- Context**

When Cortex recalls information, it should return a structured payload that the UI can render as:  
\[Alex\] \- Decided to use Tailwind for the UI on 2026-03-24 (Ref: PR \#42).  
This is achieved by tagging every SQLite record with a caller\_identity block:

* user\_id: Cryptographic hash of the human contributor.  
* agent\_id: The specific model instance (e.g., claude-3-7-sonnet-12345).  
* trace\_id: Link to the specific PostToolUse event that generated the memory.

## **3\. High-Performance Concurrency: Solving the Single-Writer Bottleneck**

A major risk of "hamstringing the program" occurs when multiple users upload information simultaneously to a shared SQLite database. Standard SQLite has a single-writer limitation.

### **The Fixed-Size Write-Queue Strategy**

To prevent SQLITE\_BUSY errors and performance degradation, Cortex must implement a **Single-Writer-Multiple-Reader** (SWMR) pattern in Rust:

1. **Serialization**: All incoming cortex\_store() requests from different users are queued into a single mpsc (Multi-Producer, Single-Consumer) channel.  
2. **Dedicated Writer**: A single background thread drains this queue and executes writes to the database. This eliminates lock contention between processes.  
3. **WAL Mode Persistence**: Enabling Write-Ahead Logging (PRAGMA journal\_mode \= WAL) allows readers (retrieval tasks) to continue unhindered even while the writer thread is committing new memories.

### **Performance Benchmarks (SQLite vs. Concurrency)**

| Scenario | Write Speed (Rows/sec) | Latency (P99) |
| :---- | :---- | :---- |
| Naive Multi-Writer | \~2,586 | 182,000ms (High failure) |
| Single-Writer Queue | **\~60,061** | **82ms** |

## **4\. Resolving Semantic Conflicts in Team Environments**

Conflicts arise when different users or agents provide overlapping but contradictory facts (e.g., User A: "Deadline is Oct 1"; User B: "Deadline is Oct 15").

### **The Semantic Merge Protocol**

Instead of simple timestamp-based overwriting, Cortex should implement a **Semantic Rebase** 1:

1. **Similarity Gating**: New writes are checked against existing memories using a threshold (e.g., $\\tau \> 0.92$).  
2. **Conflict Detection**: If a new fact overlaps but contradicts a previous one, it is flagged as a "Conflict State".2  
3. **Resolution Fragments**: The system can either:  
   * **Keep both**, tagging them as conflicting\_with: for a human to resolve.  
   * **Trigger a Verifier Agent** to check the latest project files to determine which fact is current.  
   * **Auto-Update** if the new fact comes from a human user with higher "Trust Score" than the AI agent that provided the original fact.

## **5\. Security and Governance: Asymmetric Permission Graphs**

In enterprise settings, different users have access to different resources. Cortex must ensure that an agent serving User A does not retrieve "Private" tier memories belonging to User B.

* **Bipartite Access Graphs**: Permissions are modeled as a graph linking (User, Agent, Resource). A memory fragment is only returned if the retrieval path respects the intersection of the user's and the agent's permissions.  
* **Attestation-Based Auth**: Cortex should use **AIP (Agent Identity Protocol)** to verify that the agent requesting memory is indeed authorized by the specific human user.

## **Top 10 Things to Implement for Collaborative Cortex**

| Priority | Implementation Feature | Effort | Impact | Technical Algorithm/Description |
| :---- | :---- | :---- | :---- | :---- |
| **1** | **Single-Writer MPSC Queue** | Low | Critical | Serialize all writes via a single Rust thread to eliminate SQL lock contention. |
| **2** | **Provenance Metadata Tags** | Low | High | Tag records with user\_id, agent\_id, and commit\_hash for attribution. |
| **3** | **Tiered Access Control** | Medium | Critical | Filter retrieval by (Private, Team, Global) scopes based on AAT tokens. |
| **4** | **Semantic Rebase Loop** | High | High | Detect $\\tau \> 0.92$ overlap/conflicts and trigger verifier-led resolution.1 |
| **5** | **Trust-Weighted Ranking** | Medium | Medium | Weight retrieval: $Score \= (Base \\times UserTrust \\times Recency)$. |
| **6** | **Write-Ahead Logging (WAL)** | Low | High | Enable journal\_mode \= WAL for concurrent reader-writer support. |
| **7** | **AIP Token Auth** | High | High | Implement Ed25519-signed identity tokens to prevent memory poisoning. |
| **8** | **Evidence citations in prompt** | Low | High | Return JSON with \[Alex\] \- fact for agent reasoning grounding. |
| **9** | **Datalog Scope Policy** | Medium | Medium | Use Datalog rules to restrict agents to specific repository sub-directories. |
| **10** | **Conflict Awareness Dashboard** | High | Low | UI for humans to resolve "Conflict States" manually via a diff-like view. |

#### **Works cited**

1. Solving Semantic Conflicts in Multi-Agent Systems via Delta-CAS & Semantic Rebase : r/LangChain \- Reddit, accessed March 29, 2026, [https://www.reddit.com/r/LangChain/comments/1s5vokm/solving\_semantic\_conflicts\_in\_multiagent\_systems/](https://www.reddit.com/r/LangChain/comments/1s5vokm/solving_semantic_conflicts_in_multiagent_systems/)  
2. Supervised Semantic Similarity-based Conflict Detection Algorithm: S3CDA \- arXiv.org, accessed March 29, 2026, [https://arxiv.org/html/2206.13690v2](https://arxiv.org/html/2206.13690v2)  
3. Conflict Resolution Playbook: How Agentic AI Systems Detect, Negotiate, and Resolve Disputes at Scale \- Arion Research LLC, accessed March 29, 2026, [https://www.arionresearch.com/blog/conflict-resolution-playbook-how-agentic-ai-systems-detect-negotiate-and-resolve-disputes-at-scale](https://www.arionresearch.com/blog/conflict-resolution-playbook-how-agentic-ai-systems-detect-negotiate-and-resolve-disputes-at-scale)