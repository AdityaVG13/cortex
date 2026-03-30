The realization of the Cortex memory daemon as a Tauri-based Rust application establishes a critical foundation for multi-agent coordination. However, the transition from a "shared database" to a production-ready "shared persistent brain" requires addressing the structural gap between ephemeral agent logic and stable project environments. In distributed engineering workflows, agents face three existential challenges: the **Confused Deputy** problem (writing to memories without verified identity), **Memory Drift** (retrieving facts that are invalid on the current git branch), and **Opaque Recall** (acting on memories without understanding their reasoning provenance). This report synthesizes 50+ sources to define the architecture for Agent Identity, Workspace Integrity, and Verifiably Trusted Retrieval.

## **1\. Security: Agent Identity and Attestation (AIP)**

Current multi-agent protocols like MCP and A2A facilitate communication but fail to verify *who* is communicating; a scan of approximately 2,000 MCP servers found that every single one lacked authentication. For a local daemon like Cortex, this creates a vulnerability where a prompt-injected agent can poison the shared memory of a high-trust process.

### **Invocation-Bound Capability Tokens (IBCTs)**

Cortex must implement the **Agent Identity Protocol (AIP)** using **Invocation-Bound Capability Tokens (IBCTs)**. These tokens fuse identity, attenuated authorization, and provenance into a single append-only chain.

* **Compact Mode**: Uses signed JSON Web Tokens (JWT) for single-hop interactions, taking \~0.049ms to verify in Rust.  
* **Chained Mode**: Uses **Biscuit tokens** with embedded **Datalog policies** for multi-hop delegation. When an orchestrator delegates to a specialist, it appends a "delegation block" that can only narrow the scope (e.g., restricting write access to a specific directory), never widen it.  
* **Attestation**: Verification is handled at the execution layer (the Cortex daemon), which remains outside the influence of the LLM. This prevents "Confused Deputy" attacks by ensuring each sub-agent acts with a strict subset of its parent's authority.

| Feature | Protocol/Standard | Technical Mechanism | Impact |
| :---- | :---- | :---- | :---- |
| **Identity Verification** | AIP / IBCT | Ed25519 signatures over JSONL | Prevents unauthorized memory writes. |
| **Scope Attenuation** | Biscuit / Datalog | Logic-based policy evaluation | Limits sub-agent blast radius. |
| **Attestation Binding** | AIP | DNS-based (aip:web:) or key-based IDs | Cryptographic proof of agent origin. |

## **2\. Workspace Integrity: Git-Integrated Persistence**

A primary failure mode in multi-agent coding is retrieving a decision that was true on the main branch but has been superseded in a feature branch—a state known as "Context Drift".

### **The Context Repository Pattern (Beads/Squad)**

Inspired by Steve Yegge’s **Beads** and GitHub’s **Squad**, Cortex should adopt a hybrid storage model where the git repository itself is the source of truth.

* **Repository-Native Storage**: High-level decisions and task graphs are stored as versioned **JSONL** files in a .cortex/ (or .beads/) directory. This allows agent context to branch and merge alongside the code.  
* **Dolt-Powered read-model**: While JSONL is the source of truth, Cortex can use **Dolt** (Git-for-Data) or a SQLite cache that "hydrates" from the repo to provide fast SQL querying for the agents.  
* **Branch-Aware Filtering**: Every query to localhost:7437 must include the current HEAD commit hash. Cortex filters retrieval results to ensure only memories from the current branch or its ancestors are presented as "truth".

## **3\. Collaborative Concurrency: AWCP and TODO-Claim**

When multiple agents (Claude, Codex, Local LLM) connect to Cortex, they risk "Semantic Conflicts"—where two agents work on the same file in incompatible ways.

### **AWCP: Agent Workspace Collaboration Protocol**

Cortex should act as the **Delegator Service** in an **AWCP** architecture.

* **Workspace Projection**: Instead of every agent cloning the repo, Cortex "projects" a scoped view of the filesystem to the "Executor" agents using a transport plane (e.g., SSH \+ FUSE or HTTP \+ ZIP).  
* **TODO-Claim Protocol**: To prevent parallel speedups from becoming parallel chaos, implement a **TODO-claim** protocol using **Conflict-Free Replicated Data Types (CRDTs)**.  
  1. **Scan**: Agents read the shared task board.  
  2. **Claim**: An agent writes assignedTo: agentId to a pending task.  
  3. **Verify**: After a brief sync delay (\~50ms), the agent re-reads the state. Only the winner of the deterministic convergence proceeds.

## **4\. Verifiability: Reasoning Inference Chain Retrieval (RICR)**

Retrieval accuracy degrades as context windows increase ("context rot").1 Simple vector similarity often returns snippets without the "why".

### **RICR and Generative Semantic Workspaces (GSW)**

To build a "shared brain," Cortex needs **Reasoning Inference Chain Retrieval (RICR)**.

* **GSW Representation**: Memories are not just text chunks; they are **Generative Semantic Workspaces** composed of entity nodes (e.g., "Auth System"), event nodes ("Migration"), and QA pairs that form edges.  
* **Beam-Search Retrieval**: When a query arrives, Cortex decomposes it and follows "reasoning chains" through the semantic graph (e.g., *Fact A \-\> leads to \-\> Decision B*) rather than returning isolated hits.  
* **Just-in-Time Verification**: Like GitHub Copilot’s memory, every retrieved fact must include **Provenance Links** (citations) to specific code locations or commit hashes.2 Agents are then prompted to verify these citations against the *current* state of the filesystem before acting.2

## **Top 10 Things to Implement for Cortex (Phase 4\)**

| Priority | Implementation Feature | Effort | Impact | Technical Algorithm/Description |
| :---- | :---- | :---- | :---- | :---- |
| **1** | **AIP Auth Middleware** | Medium | Critical | Validate AAT/IBCT tokens via Ed25519. Reject unauthenticated tools/call. |
| **2** | **Git-Branch Filtering** | Medium | High | Metadata check of repo branch on query to prevent cross-branch context drift. |
| **3** | **TODO-Claim Protocol** | Low | High | Atomic task-assignment logic in SQLite/CRDT to prevent agent race conditions. |
| **4** | **RICR Retrieval Engine** | High | High | Beam-search traversal of semantic graph for multi-hop evidence. |
| **5** | **Provenance Citations** | Low | High | Map facts to COMMIT\_HASH; force agents to verify code exists before acting.2 |
| **6** | **Zenoh Message Bus** | High | Medium | Replace SSE with Zenoh Rust bindings for high-throughput binary inter-agent transport. |
| **7** | **DLP Secrets Scrubber** | Medium | Medium | Bayesian filter to redact keys/PII before they enter the persistent brain. |
| **8** | **AWCP Workspace Mount** | High | Medium | Standardized "Delegator" framework to project files to "Executor" agents. |
| **9** | **Human-in-the-Loop Hub** | Medium | Medium | OS-level approval dialogs for low-confidence memory writes ($\\tau \< 0.55$). |
| **10** | **Zettelkasten Linking** | High | Low | Dynamic link generation between notes based on shared entity attributes. |

#### **Works cited**

1. Effective context engineering for AI agents \- Anthropic, accessed March 29, 2026, [https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)  
2. Building an agentic memory system for GitHub Copilot, accessed March 29, 2026, [https://github.blog/ai-and-ml/github-copilot/building-an-agentic-memory-system-for-github-copilot/](https://github.blog/ai-and-ml/github-copilot/building-an-agentic-memory-system-for-github-copilot/)  
3. About GitHub Copilot coding agent, accessed March 29, 2026, [https://docs.github.com/copilot/concepts/agents/coding-agent/about-coding-agent](https://docs.github.com/copilot/concepts/agents/coding-agent/about-coding-agent)