---
name: recall
description: Search Cortex memory for past decisions, conventions, and lessons. Use when the user asks about previous work, past decisions, or what they learned about something.
---

# Cortex Recall

Search your persistent memory for relevant past knowledge.

## Usage

```
/cortex:recall <query>
```

The `$ARGUMENTS` placeholder contains your search query.

## Budget Guidance

Cortex adapts detail level to token budget:

| Query Type | Budget | Detail Level |
|------------|--------|--------------|
| Specific lookup | 200 | Balanced - key facts + context |
| Broad exploration | 500 | Full detail - complete entries |
| Quick check | 0 | Headlines only - fast preview |

## How to Use

1. **Formulate your question:**
   - "What did I decide about database choice?"
   - "Error handling conventions"
   - "Windows compatibility issues"

2. **Run the skill:**
   - Cortex will search and return relevant memories

3. **Follow up:**
   - If too broad, narrow with more specific terms
   - If too specific, broaden the query

## Example Queries

- `async rust framework choice` - Architecture decision
- `error handling patterns` - Coding conventions
- `windows file path issue` - Bugfix lesson
- `authentication approach` - Security decision

## Integration

This skill uses the `cortex_recall` MCP tool with your `$ARGUMENTS` as the query.
