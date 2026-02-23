# Reading papers is broken

I've spent a lot of time reading research papers. Probably too much time.

The thing about papers is they're dense. Even if you're good at reading them — you know how to skim, you skip the parts you don't need — there's just a lot to hold in your head. The method section alone might have architecture details, training procedures, loss functions, specific hyperparameter choices, and you're trying to figure out which of those details matter for what *you're* working on. Then there's ablations that might be relevant later, implementation details you'll need if you actually try to reproduce it, connections to other work you've read.

You can get through a paper in 30 minutes if you know what you're doing. But actually retaining all of it? Knowing which details to care about when you come back to it two weeks later? That's the hard part. And when you're reading multiple papers on the same topic, trying to understand how different approaches compare — which design decisions are actually different vs. just presented differently — that's where it really breaks down. You can't keep five papers in working memory at once.

Then there's everything around the reading. You're in a PDF, you need to check a reference, so you open Semantic Scholar. You find a related paper, open that too. Now you're cross-referencing, taking notes somewhere else, checking a GitHub repo to see how the method was actually implemented. Two hours later you've got twelve tabs, scattered notes, and a vague feeling you understood something earlier but can't find where you wrote it down.

Most people I know have tried the obvious fix — paste the paper into ChatGPT or Claude, ask it to explain things. Works fine for a quick summary. Falls apart for real research. You ask a follow-up and the answer shows up way below the original explanation. You lose your place. You close the tab and it's gone. These tools have memory now, sure, but it's not the kind of memory that's useful for research — you can't run a comparative analysis across five papers you read over two weeks, or search your past findings by topic. It's more like "vaguely remembers you mentioned diffusion models once."

## What I wanted

Pretty simple, honestly. I want something that reads a paper and helps me understand the parts that matter for *my* question — and keeps track of all the detail I can't hold in my head. I want to ask questions in context — not at the bottom of a chat log, but right there in the explanation. Highlight a confusing paragraph, say "what does this mean," get the answer woven in.

I want to come back next week and have it all still be there. And when I've read a bunch of papers on the same topic, I want to synthesize across them — how do these approaches actually compare, what are the real tradeoffs.

I also want it connected to my Zotero library. I've got years of papers in there — annotations, highlights, notes, collections. Every time I open a new chat with Claude or ChatGPT, none of that exists.

And I want it in my terminal, next to my code. You don't read a paper just to read it. You read it because you want to understand an approach, look at the reference implementation, maybe combine ideas from multiple papers and codebases into something new. That workflow lives in the terminal, not a browser tab.

So we built `ata`.

## How it works

`ata` is an AI agent that runs in your terminal. You give it a topic or point it at a paper. It searches Semantic Scholar, arXiv, and OpenAlex at the same time, traces citation graphs, and comes back with papers grouped by approach — not a flat list but a map of the field. It can search patents too if that's relevant to what you're looking at.

When you go deep on a paper, it reads the full PDF and pulls out what actually matters — the core method, key decisions, specific numbers. It presents this in a reading view inside the terminal. Navigable sections, foldable detail blocks, content that streams in as you read.

The thing I care about most: you ask follow-up questions without leaving the document. You're reading, something doesn't click, you type a question, and the agent updates that section in place. You can highlight a specific passage and ask about just that part. The document evolves as you interact with it.

## It connects to your stuff

`ata` talks directly to Zotero — cloud API or the local app. It sees your papers, collections, tags, annotations, notes, group libraries. Your whole reference library becomes something the agent can actually work with.

It also searches Hacker News for what practitioners think about the approaches you're researching. Different signal than papers, but useful when you want to know what actually works in production vs. what works on a benchmark.

## It doesn't forget

Every synthesis gets saved as a knowledge card. Structured, tagged, searchable. A journal tracks what you explored. A context file picks up on your priorities so the agent gets better over time at surfacing what's relevant to you.

When you've got multiple papers saved, you can run comparative analysis across them. Or quick briefings to orient yourself before diving deep. Each paper you read makes the next one more useful.

`ata` is open source. [Give it a try.](https://github.com/Agents2AgentsAI/ata)
