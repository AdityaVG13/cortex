---
name: store
description: Store a decision or lesson in Cortex persistent memory. Use when the user wants to save a decision, convention, lesson learned, or important knowledge for future sessions.
---

# Cortex Store

Save a decision or lesson to your persistent memory.

## Usage

```
/cortex:store <decision>
```

The `$ARGUMENTS` placeholder contains the decision/lesson text.

## What to Store

Good candidates for storage:

- **Architecture decisions**: "Chose SQLite for local-first, embedded use case"
- **Coding conventions**: "Use conventional commits: feat:, fix:, docs:"
- **Lessons learned**: "NTFS file locking requires LockFileEx on Windows"
- **Project knowledge**: "API rate limit is 100 req/min, use exponential backoff"
- **Team agreements**: "All PRs require at least one review"

## Confidence Score Guidance

When storing, include confidence:

| Score | Meaning |
|-------|---------|
| 1.0 | Definitive - confirmed by testing/production |
| 0.9 | High confidence - validated approach |
| 0.8 | Solid - good evidence |
| 0.7 | Reasonable - working assumption |
| 0.5 | Tentative - needs more validation |

## Context

Always include context - the "why" behind the decision:

- What problem did it solve?
- What alternatives were considered?
- What tradeoffs were made?

## Example

Input:
```
/cortex:store "Use path.join() for all file paths - hardcoded / or \\ breaks Windows compatibility"
```

Stored as:
- **Decision**: Use path.join() for all file paths
- **Context**: Hardcoded / or \\ breaks Windows compatibility
- **Confidence**: 0.9

## Integration

This skill uses the `cortex_store` MCP tool with your `$ARGUMENTS` as the decision text.
