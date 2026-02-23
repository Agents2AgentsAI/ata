# Chat is the wrong interface for long AI output

We're building an AI agent that reads research papers and synthesizes them. Early on we had a problem: the best output was long and structured, but we were showing it in a chat window where it just scrolled away.

You ask an AI to explain a paper. The explanation is good. You have a follow-up question. The answer shows up at the bottom of the conversation, disconnected from the section it's about. Three follow-ups later, your understanding is scattered across messages. The more questions you ask, the harder it is to find anything.

So we built a reading view — a structured document with navigable sections. But the decision that actually mattered: follow-up questions modify the document in place instead of creating new messages. You're reading, something's confusing, you ask right there, and the answer gets woven into the section. Every question makes the document better instead of making the chat log longer.

The insight is simple: when AI output is a document, the interface should be a document. And when you ask questions about it, the answers should improve it — not create a separate conversation about it.

We built this into `ata`, an open-source research agent: [github.com/Agents2AgentsAI/ata](https://github.com/Agents2AgentsAI/ata)
