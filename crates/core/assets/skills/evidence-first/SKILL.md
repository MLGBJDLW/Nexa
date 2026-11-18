---
name: evidence-first
description: Activate for every answer that uses knowledge base search results or cites documents. Trigger when the response relies on retrieved sources, facts from indexed files, or any claim that requires citation, synthesis, or confidence calibration across multiple documents.
---

## Trigger
Every answer that uses knowledge base search results.

## Rules
1. Place `[cite:CHUNK_ID]` next to each claim supported by an indexed local source. Reuse the ID returned for that passage. For a direct file read, copy its suggested citation. If you mention a page, line, paragraph or cell in prose, use the coordinates returned with the evidence. PDF extracted line numbers refer to text within that page. For extracted-only evidence, name the available section.
2. When multiple sources exist:
   - If they AGREE: synthesize into one answer, cite all sources
   - If they CONFLICT: present both views explicitly, note the contradiction
   - If only ONE source: clearly state the answer comes from a single source
3. Confidence levels:
   - HIGH: 3+ sources agree → state confidently
   - MEDIUM: 1-2 sources → note limited evidence
   - LOW: no direct source, inferring → explicitly say "Based on inference, not direct knowledge base evidence"
4. Never fabricate information not in the search results
5. If the knowledge base has NO relevant results, say so clearly — don't guess

## Format
📚 **Sources:** [Document1], [Document2]
[Answer with inline citations]

💡 **Confidence:** HIGH/MEDIUM/LOW — [reason]
