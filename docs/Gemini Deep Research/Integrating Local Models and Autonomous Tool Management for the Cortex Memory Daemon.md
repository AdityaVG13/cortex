The architecture of Cortex as a localized Tauri-based daemon provides a unique opportunity to offload high-frequency cognitive tasks—such as memory extraction, conflict detection, and context compression—to local Large Language Models (LLMs). By utilizing models like Qwen-2.5-Coder (1.5B) and Phi-4 (14B) on-device, Cortex can act as a "cognitive kernel" that maintains a shared persistent brain without the latency or privacy risks of cloud-based inference. This report details the technical mechanisms for local model connectivity, background compression pipelines, and the autonomous tool-equipping of models to interact with repository state.

## **1\. Local-First Integration: The Thalamus-Synapse Model**

To enable seamless interaction between heterogeneous agents and local models, Cortex should implement a **Thalamus-Synapse architecture**. In this model, the Rust daemon (Thalamus) acts as the central router, while local models are spawned as dedicated workers or accessed via a standardized protocol.

### **Connectivity via Model Context Protocol (MCP)**

The most robust solution for connecting any AI (Claude Code, Gemini CLI, or a local process) to Cortex is using the **Model Context Protocol (MCP)**. Cortex should expose itself as a stdio-based or SSE-based MCP server \[9, S\_R120\].

* **Stdio Synapse:** The local model is spawned as a subprocess. The parent daemon pipes JSON-RPC 2.0 messages over standard I/O \[9, S\_R103\].  
* **Tauri Sidecar:** Leveraging Tauri’s sidecar feature, local inference engines like **Ollama** or **mistral.rs** can run alongside the daemon, providing a unified installer for the user.  
* **Performance:** A dedicated background thread for models like Qwen-2.5-Coder (1.5B) can achieve throughputs of $\\sim 100$ tokens/s on modern consumer hardware, preventing inference from becoming a bottleneck during background tasks.1

## **2\. Background Cognitive Compression Pipelines**

To solve the "token tax" problem, Cortex must use local models to autonomously manage the shared brain's state through periodic consolidation.

### **The Sawtooth Pruning Pattern (Focus Architecture)**

Inspired by biological exploration strategies, the **Focus Architecture** enables "intra-trajectory compression" where the model actively prunes its own history.

1. **Start Focus:** The agent marks a checkpoint (e.g., "Investigating Auth Bug").  
2. **Explore:** The agent performs raw tool calls and observations.  
3. **Consolidate:** The agent invokes complete\_focus(). A local model (e.g., Qwen-2.5-Coder) summarizes the learnings and outcomes into a structured "Knowledge" block.  
4. **Withdraw:** The raw logs between the checkpoints are deleted from the active context and stored in the episodic long-term store.  
* **Impact:** This method achieves a 22.7% token reduction in software engineering tasks with zero loss in accuracy.

### **Hierarchical Consolidation via TiMem**

Cortex can implement a **Temporal Memory Tree (TMT)** using local models to abstract information as it ages.

* **Tier 1 (Raw Traces):** Append-only log of every interaction.  
* **Tier 2 (Episodic Nodes):** Summarized episodes of related turns.  
* **Tier 3 (Semantic Nodes):** High-level project invariants and user preferences extracted by a local model using an **ADD/UPDATE/DELETE** state machine \[5, S\_R195\].

## **3\. Autonomous Tool-Building and System Access**

For local models to effectively manage the workspace, they must be equipped with system-level capabilities. Since they often lack pre-built integrations, Cortex must provide a mechanism for **Dynamic Tool Synthesis**.

### **Workspace Delegation via AWCP**

The **Agent Workspace Collaboration Protocol (AWCP)** provides the missing layer at the agent-workspace boundary.

* **On-site Access:** Instead of just exchanging messages, AWCP allows the Delegator (Cortex) to project a scoped view of the filesystem to the Executor (the local model).  
* **Files-as-Interface:** The local model uses unmodified local toolchains (e.g., ls, git, grep) within the delegated workspace.  
* **Tool Discovery:** Local models discover available tools via **Agent Cards**—JSON metadata files that advertise endpoints and security requirements.2

### **Grammar-Based Sampling for Tool Reliability**

To ensure that local models (which may be less reliable than GPT-4o) produce valid JSON tool calls, Cortex should implement **Grammar-Based Sampling** at the inference layer.

* **Implementation:** Using crates like candle-core or ort with JSON Schema enforcement ensures that the model's output always adheres to the expected function signature.

## **4\. Local Model Selection and Inference Runtimes**

The choice of model and runtime is critical for a Rust-native daemon running on localhost:7437.

| Model / Runtime | Best Use Case | Performance Metric |
| :---- | :---- | :---- |
| **Qwen-2.5-Coder 1.5B** | Real-time fact extraction & "Observer" role. | 100+ tok/s on CPU/GPU.1 |
| **Phi-4 14B** | Complex reasoning & "Reflector" role. | High consistency on complex engineering tasks.4 |
| **Candle (Rust)** | Pure-Rust inference, minimal binary size. | Native HuggingFace integration, ideal for Tauri. |
| **ONNX Runtime (ort)** | Production-grade inference with SIMD/GPU support. | 3-5x faster than Python-based inference. |

## **Top 10 Things to Implement for Local Model Integration**

| Priority | Feature | Effort | Impact | Technical Algorithm/Description |
| :---- | :---- | :---- | :---- | :---- |
| 1 | **MCP stdio Hub** | Low | Critical | Spawn local models as subprocesses using the MCP standard for zero-latency tool use. |
| 2 | **Sawtooth Focus Tools** | Medium | Critical | Implement start\_focus and complete\_focus tools to enable agent-controlled context pruning. |
| 3 | **Qwen-2.5-Coder Worker** | Medium | High | A dedicated background thread for the 1.5B model to perform "asynchronous dreaming" (consolidation). |
| 4 | **AWCP Workspace Mount** | High | High | Project-scoped filesystem delegation allowing local models to run git and ls directly. |
| 5 | **Grammar-Constrained Sampling** | High | High | Use ort or candle to enforce JSON Schema on model outputs, ensuring valid tool calls. |
| 6 | **FSRS-6 Memory Decay** | Medium | High | Implement a power-curve decay for memory retrievability using the same algorithm as Anki \[10, S\_R303\]. |
| 7 | **SIMD Similarity Kernels** | Low | Medium | Use SimSIMD for sub-millisecond retrieval over 100k+ vector nodes. |
| 8 | **Triple-Date Anchoring** | Low | Medium | Tag every memory with Created, Referenced, and Relative dates for temporal reasoning.5 |
| 9 | **Just-in-Time Citation Check** | Medium | Medium | Force agents to verify memory provenance against local git HEAD before acting \[8, S\_R56\]. |
| 10 | **Datalog Scope Attenuation** | High | Medium | Use Biscuit tokens to ensure local models have restricted write-access to shared memory. |

## **Sources Used in Report**

,,,,, \[8\],, \[9\],,,,,,,,,, \[6\],,, \[5\], 1, \[3\],,, \[10\], \[2\],,,,, 4,,,.

## **Additional Sources (Unused)**

1. 7  
   \- Context engineering policies for constructing $C\_t$.  
2. \*\*\*\* \- Persistent knowledge graphs with MCP server.  
3. \*\*\*\* \- Axum SSE coordination for multi-agent events.  
4. \*\*\*\* \- Beads: Distributed git-backed issue tracker.  
5. \*\*\*\* \- CodeCRDT: Observation-driven coordination.  
6. \*\*\*\* \- Shared business brain implementation patterns.  
7. \*\*\*\* \- Rust context engineering and CrateDoc tools.  
8. \*\*\*\* \- Zenoh migration guide for async Rust.  
9. \*\*\*\* \- Convergio: 9 Gates of code quality automation.  
10. \*\*\*\* \- Git-free usage patterns for Beads.

#### **Works cited**

1. Benchmark comparison: Qwen3-Coder-Next vs DeepSeek V3.2 vs Minimax M2.5 \- Reddit, accessed March 29, 2026, [https://www.reddit.com/r/DeepSeek/comments/1r4jf2b/benchmark\_comparison\_qwen3codernext\_vs\_deepseek/](https://www.reddit.com/r/DeepSeek/comments/1r4jf2b/benchmark_comparison_qwen3codernext_vs_deepseek/)  
2. A Breakdown of A2A, MCP, and Agentic Interoperability : r/LLMDevs \- Reddit, accessed March 29, 2026, [https://www.reddit.com/r/LLMDevs/comments/1lq6uxn/a\_breakdown\_of\_a2a\_mcp\_and\_agentic/](https://www.reddit.com/r/LLMDevs/comments/1lq6uxn/a_breakdown_of_a2a_mcp_and_agentic/)  
3. MCP vs A2A in Practice: A Developer's Guide to Composing Agent Protocols \- Adopt AI, accessed March 29, 2026, [https://www.adopt.ai/blog/mcp-vs-a2a-in-practice](https://www.adopt.ai/blog/mcp-vs-a2a-in-practice)  
4. DeepSeek-V3, GPT-4, Phi-4, and LLaMA-3.3 generate ... \- arXiv, accessed March 29, 2026, [https://arxiv.org/pdf/2502.14926](https://arxiv.org/pdf/2502.14926)  
5. Observational Memory: 95% on LongMemEval \- Mastra Research, accessed March 29, 2026, [https://mastra.ai/research/observational-memory](https://mastra.ai/research/observational-memory)  
6. Announcing Observational Memory \- Mastra Blog, accessed March 29, 2026, [https://mastra.ai/blog/observational-memory](https://mastra.ai/blog/observational-memory)  
7. Context Engineering in Agent. Memory Patterns Core principles and… \- Medium, accessed March 29, 2026, [https://medium.com/agenticais/context-engineering-in-agent-982cb4d36293](https://medium.com/agenticais/context-engineering-in-agent-982cb4d36293)  
8. Building an agentic memory system for GitHub Copilot, accessed March 29, 2026, [https://github.blog/ai-and-ml/github-copilot/building-an-agentic-memory-system-for-github-copilot/](https://github.blog/ai-and-ml/github-copilot/building-an-agentic-memory-system-for-github-copilot/)  
9. MCP vs A2A: Compare Single-Agent & Multi-Agent Protocols, accessed March 29, 2026, [https://www.truefoundry.com/blog/mcp-vs-a2a](https://www.truefoundry.com/blog/mcp-vs-a2a)  
10. AI memory is useful, but only if it goes beyond storing facts : r/artificial \- Reddit, accessed March 29, 2026, [https://www.reddit.com/r/artificial/comments/1rfhs9h/ai\_memory\_is\_useful\_but\_only\_if\_it\_goes\_beyond/](https://www.reddit.com/r/artificial/comments/1rfhs9h/ai_memory_is_useful_but_only_if_it_goes_beyond/)