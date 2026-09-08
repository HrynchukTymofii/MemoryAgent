# Personal Memory OS — Product & Technical Specification

## 0. Executive summary

### Product thesis

Build a **local-first Personal Memory OS / Personal Command Center** that makes saving, retrieving, organizing, and acting on information almost frictionless.

The central interaction is:

> **Global shortcut → speak naturally → the system understands the context → performs the action → disappears.**

The product is not primarily an AI chatbot, a passive computer recorder, or a replacement for Notion.

It is a **low-friction control layer between the user and their computer**.

Examples:

- “Remember this and put it under Programming → React.”
- “Save this article; I want to read it later.”
- “Remind me about this next Tuesday.”
- “Find that article I read last week about React state.”
- “Show me everything I saved about fertilizers.”
- “Download these photos, put them in a folder called Demo Day 2026, and keep the original filenames.”
- “Find the photos from the accelerator event where I was on stage.”
- “What did I save about customer acquisition cost?”
- “Give me a five-minute quiz on the business metrics I saved this week.”

The product should make these actions so easy that **not using it feels harder than using it**.

---

# 1. Core problem

Modern users accumulate information faster than they can organize or retrieve it.

Typical examples:

- hundreds of browser tabs
- articles saved but never revisited
- useful quotes that disappear from memory
- PDFs and research papers scattered across folders
- photos from events with unclear locations
- job applications spread across browser tabs and files
- notes in multiple applications
- useful websites that are difficult to find again
- concepts learned temporarily and then forgotten
- tasks that exist only in the user's head

The problem is not primarily lack of storage.

The problem is **friction**.

Saving something currently requires some combination of:

1. Open an application.
2. Find the correct folder.
3. Create a note/page.
4. Copy the information.
5. Rename it.
6. Add tags.
7. Organize it.
8. Remember where it was placed.

For many pieces of information, the user decides that this is too much work and does nothing.

The product should invert this:

> **The user decides what matters. The system does the organizational work.**

---

# 2. Product principles

## Principle 1 — Capture must be nearly instantaneous

The primary interaction should require:

- one global shortcut
- a short voice command
- immediate execution

The user should not need to open a large application.

## Principle 2 — The AI should reduce work, not create work

The system should not require the user to explain everything.

It should infer context from:

- active application
- active window
- selected text
- clipboard
- current URL
- current file
- current folder
- recent conversation
- existing knowledge structure

## Principle 3 — User-controlled memory

The system should NOT automatically record everything the user does.

The user explicitly decides what becomes durable memory.

This differentiates the product from passive computer-memory systems.

## Principle 4 — Local-first

Core functionality should work without an internet connection.

Cloud AI should be an optional provider, not a mandatory dependency.

## Principle 5 — Fast before clever

A smaller model that responds in 200–500 ms is often preferable to a more intelligent model that takes several seconds.

Use large models for difficult tasks; use deterministic code and small local models for routine tasks.

## Principle 6 — Structured data first, RAG second

The system should not be “a vector database with a chatbot.”

It should have a real data model containing:

- knowledge items
- sources
- collections
- tasks
- reminders
- files
- relationships
- events
- commands
- provenance

Embeddings are one retrieval mechanism, not the entire database.

## Principle 7 — Never silently destroy or misplace user data

Destructive or ambiguous actions require confirmation.

Examples:

- “Delete these files” → confirmation.
- “Put this somewhere with my house stuff” → ask if confidence is low.
- “Save this to React” → execute immediately if confidence is high.

## Principle 8 — The system should become personal

Over time it should learn:

- the user's terminology
- preferred categories
- common destinations
- command patterns
- frequently used actions
- personal knowledge relationships

---

# 3. Product identity

Recommended category:

**Personal Memory OS**

Alternative:

**Personal Knowledge Assistant**

The product should not primarily be marketed as:

- AI note-taking
- RAG chatbot
- second brain
- AI Notion
- AI search

Potential positioning:

> **Capture once. Find forever.**

or:

> **Tell your computer what to remember.**

or:

> **Your computer's memory, without the maintenance.**

---

# 4. Primary users

Initial target:

- entrepreneurs
- developers
- researchers
- technical professionals
- students
- creators
- people managing many parallel projects
- people with information overload
- people who frequently research multiple unrelated domains

Typical user:

> “I have 200 tabs open because I don't want to lose useful information, but I don't have time to organize them.”

---

# 5. Core user experience

## 5.1 Global command

Example:

`Ctrl + Space`

A tiny overlay appears.

The user speaks:

> “Save this to programming React. The important part is the explanation of state as a snapshot.”

The system captures current context:

- active application
- URL
- page title
- selected text
- clipboard
- timestamp

Then performs the action.

Overlay disappears.

Target interaction time:

**a few seconds.**

---

# 6. Core command types

## SAVE

“Save this to Programming → React.”

## SAVE + REMINDER

“Save this and remind me next Sunday.”

## SEARCH

“Find the article about React state I read last week.”

## SHOW

“Show everything I saved about fertilizers.”

## OPEN

“Open the React article I saved yesterday.”

## MOVE

“Move this to Business → Metrics.”

## TAG

“Tag this as job search.”

## NOTE

“Remember that my router uses this configuration.”

## TASK

“Create a task to apply for this job tomorrow.”

## PLAN

“Plan my afternoon around these three tasks.”

## FILE

“Download these photos and put them in a folder called Demo Day 2026.”

## RESEARCH

“Find resources about local LLM inference and save the useful ones.”

## EXPLAIN

“Explain this using what I already saved about React.”

## LEARN

“Quiz me on the business metrics I saved this week.”

---

# 7. Context acquisition

The desktop client should be able to expose a normalized context object.

Example:

```json
{
  "active_application": "Chrome",
  "active_window_title": "React - State as a Snapshot",
  "current_url": "https://react.dev/learn/state-as-a-snapshot",
  "selected_text": "...",
  "clipboard_text": "...",
  "timestamp": "2026-09-06T20:00:00"
}
```

Other possible context sources:

- selected files
- active folder
- current document
- screenshot
- browser tab
- clipboard image
- microphone
- filesystem
- calendar
- email
- supported third-party apps

Context acquisition must be permission-aware.

---

# 8. Knowledge model

A knowledge item should contain both structured information and semantic information.

Conceptual schema:

```text
KnowledgeItem
├── id
├── title
├── content
├── summary
├── source
├── source_type
├── collection_id
├── tags[]
├── created_at
├── updated_at
├── captured_at
├── last_accessed_at
├── relationships[]
├── embedding
└── metadata
```

Source types:

- webpage
- highlighted text
- voice note
- PDF
- image
- file
- manual note
- conversation
- external document

---

# 9. Knowledge organization

The UI may present a hierarchy:

```text
Study
├── Programming
│   ├── React
│   ├── TypeScript
│   └── Python
├── English
├── Business
└── AI

Life
├── House
├── Garden
└── Finance

Career
├── Job Applications
├── Companies
└── Interviews
```

However, the underlying data model should not be a rigid tree.

Use:

- collections
- tags
- many-to-many relationships
- references

Example:

```text
React State Article
    ├── collection → Study / Programming / React
    ├── related_to → React Hooks
    ├── related_to → Interview Preparation
    └── source → react.dev
```

This prevents information from being trapped in one folder.

---

# 10. Provenance

Every important piece of information should preserve its origin.

Example:

```text
React State as a Snapshot

Captured:
September 2, 2026

Source:
react.dev/learn/state-as-a-snapshot

Method:
Highlighted text + voice command

Collection:
Study / Programming / React

Related:
React Hooks
useState
Rendering
```

This improves trust and allows the user to reopen the original source.

---

# 11. Search architecture

Do not rely on vector search alone.

Use **hybrid retrieval**.

## 11.1 Keyword search

Useful for exact terms:

- React
- CAC
- LTV
- company name
- URL
- person

## 11.2 Semantic search

Useful for meaning.

Example:

User:

> “Find what I saved about state not changing during rendering.”

Saved document:

> “State is a snapshot for each render.”

Embeddings allow the two concepts to match despite different wording.

## 11.3 Metadata filtering

Examples:

- collection = Programming
- source = react.dev
- date = last 30 days
- type = article

## 11.4 Temporal retrieval

Examples:

- “last Tuesday”
- “something I saved last month”
- “the article I opened yesterday”

## 11.5 Relationship retrieval

Examples:

- related to React
- related to job applications
- related to a particular company

## 11.6 Ranking

Combine multiple signals:

```text
score =
    semantic_similarity
    + keyword_score
    + recency
    + metadata_match
    + relationship_score
    + source_match
```

---

# 12. What is an embedding?

An embedding is a numerical representation of the semantic meaning of a piece of information.

For example:

```text
"React state behaves like a snapshot"
                ↓
       embedding model
                ↓
[0.12, -0.72, 0.41, ...]
```

A semantically similar sentence will have a nearby vector.

This enables semantic search.

The project should support:

- remote embedding APIs
- local embedding models

The embedding model should be replaceable.

---

# 13. Database architecture

Primary database:

**PostgreSQL**

Vector extension:

**pgvector**

Main tables:

```text
users
collections
knowledge_items
sources
tags
knowledge_tags
knowledge_relationships
tasks
reminders
files
commands
command_runs
conversations
embeddings
events
```

PostgreSQL is the source of truth.

Redis is auxiliary infrastructure.

---

# 14. Redis

Redis can run locally and should be used for:

- caching
- temporary agent state
- task queues
- background jobs
- event/pub-sub
- streaming state
- rate limiting

Redis should NOT be the primary knowledge store.

---

# 15. Agent architecture

Use LangGraph for stateful agent workflows.

Conceptual flow:

```text
USER INPUT
    ↓
Speech-to-text
    ↓
Intent Router
    ↓
Context Resolver
    ↓
Tool Selection
    ↓
Execution
    ↓
Validation
    ↓
Response
```

Possible intents:

```text
SAVE
SEARCH
OPEN
MOVE
TAG
TASK
REMINDER
PLAN
FILE_OPERATION
RESEARCH
LEARN
CHAT
```

---

# 16. Tools

The agent should call explicit tools rather than having unrestricted database access.

Examples:

```text
save_memory()
search_memory()
get_memory()
move_memory()
create_task()
create_reminder()
search_files()
create_folder()
move_file()
rename_file()
download_file()
open_url()
open_file()
web_search()
create_quiz()
schedule_review()
```

This gives the agent a controlled action space.

---

# 17. Confidence system

Every action should have a confidence score.

Example:

```text
"Save this to React."

Intent confidence: 0.99
Collection confidence: 0.96
Source confidence: 1.00

→ execute automatically
```

Ambiguous example:

```text
"Put this with the house stuff."

Possible destinations:
1. House / Internet
2. House / Electricity
3. House / Automation

Confidence: 0.44

→ ask user
```

The goal is:

> **No unnecessary questions, no dangerous guesses.**

---

# 18. Speech architecture

Speech is a core feature, not a future enhancement.

Preferred pipeline:

```text
Microphone
    ↓
Voice Activity Detection
    ↓
whisper.cpp
    ↓
Streaming / final transcription
    ↓
Intent Router
```

Target:

- microphone ready before command
- model loaded in memory
- local inference
- no unnecessary network round trips
- immediate visual feedback

The product should feel closer to a keyboard shortcut than to “opening an AI app.”

---

# 19. Text-to-speech

Optional response speech:

```text
Agent response
      ↓
Fish Speech / local TTS
      ↓
Audio
```

The user should also be able to choose:

- silent UI
- text response
- spoken response
- both

---

# 20. Local model architecture

The application should have a model-provider interface.

```text
LLMProvider
├── LocalLLM
├── OpenAI
├── Anthropic
├── Gemini
└── Other provider
```

Local stack can use:

- llama.cpp
- Ollama
- other compatible local inference servers

The architecture should not assume a specific model.

---

# 21. Small personal model

A dedicated small model can eventually learn the user's command patterns.

It does NOT need to be a general-purpose LLM.

Target task:

**Personal Intent + Routing Model**

Example:

Input:

> “Put this with my React stuff.”

Output:

```json
{
  "intent": "save",
  "collection": "Study/Programming/React",
  "confidence": 0.96
}
```

The application can continuously collect examples:

```text
user command
→ system interpretation
→ user correction/approval
```

These become training data.

Later:

```text
personal command dataset
        ↓
fine-tuning / LoRA
        ↓
small local model
        ↓
fast personal routing
```

This makes LoRA a legitimate learning component of the project.

---

# 22. Research agent

A separate agent mode can search external sources.

Example:

> “Find me good resources about local LLM inference.”

Workflow:

```text
User request
    ↓
Intent
    ↓
Web search
    ↓
Fetch sources
    ↓
Extract useful content
    ↓
Rank
    ↓
Present
    ↓
User selects
    ↓
Save selected resources
```

Important:

The research agent should not automatically save everything.

The user remains the authority over durable memory.

---

# 23. File and photo assistant

This is a major extension of the same philosophy.

Problem:

> “I have 300 photos from different events, but finding and organizing the right ones takes too much effort.”

Command:

> “Download these photos, create a folder called Demo Day 2026, and save them there.”

Possible operations:

```text
create_folder()
download()
move()
copy()
rename()
tag()
group()
search()
open()
```

Another example:

> “Find the photos from the accelerator event where I was presenting.”

Potential future workflow:

```text
Natural language
    ↓
photo search
    ↓
metadata search
    ↓
visual similarity
    ↓
event/date context
    ↓
candidate photos
    ↓
user selection
```

This should be considered a **future capability**, not required for the initial product.

---

# 24. File organization philosophy

The product should avoid forcing users into a rigid filesystem.

Instead:

> “Put these in the Demo Day 2026 folder.”

The assistant handles:

- folder creation
- naming
- movement
- duplication avoidance
- optional metadata
- indexing

The user should not need to perform file-management rituals.

---

# 25. Planning system

The assistant can eventually combine:

```text
Tasks
Projects
Calendar
Deadlines
Knowledge
Reminders
Goals
```

Example:

> “What should I work on this afternoon?”

The planner retrieves:

- active tasks
- deadlines
- priorities
- calendar
- recent work
- relevant knowledge

and proposes a plan.

The user can then say:

> “Move React interview prep to tomorrow and give me two hours for Flari.”

---

# 26. Learning mode

This should be considered a **separate product surface built on the same memory engine**.

The base product stores knowledge.

The learning layer turns selected knowledge into active recall.

Example:

User saves:

```text
Customer Acquisition Cost (CAC)
Lifetime Value (LTV)
Conversion Rate
Revenue per Install
Retention
```

Then:

> “Quiz me on the business metrics I saved this week.”

System generates:

- definitions
- multiple-choice questions
- short-answer questions
- examples
- comparison questions
- application questions

Example:

> **What does CAC measure?**

User answers.

System evaluates.

Then schedules review.

---

# 27. Learning architecture

```text
Knowledge
    ↓
Concept extraction
    ↓
Learning objects
    ↓
Question generation
    ↓
Active recall
    ↓
Performance tracking
    ↓
Spaced repetition
```

Learning objects can include:

```text
Concept
Definition
Example
Counterexample
Formula
Relationship
Application
Question
```

The learning engine can use spaced repetition.

Potential scheduling algorithms:

- FSRS
- SM-2

The goal is not to turn the entire Personal Memory OS into a flashcard application.

The goal is:

> **When the user explicitly marks something as “I want to remember this,” the system can help them actually remember it.**

---

# 28. Learning mode should not be in V1

The architecture should support it.

The first product should focus on:

**Capture → organize → retrieve → act**

Later:

**Capture → organize → retrieve → learn**

This preserves product clarity.

---

# 29. Competitor landscape

## Mem

Closest to:

- personal knowledge
- AI memory
- voice
- tasks
- routines

Current pricing includes a free tier, $9/month Plus, and multiple Pro tiers; Mem also offers push-to-talk and voice mode. It is therefore a significant competitor on the “AI memory assistant” side.

Official source:
https://get.mem.ai/

Key difference:

**Your product should be capture-first and shortcut-first rather than conversation-first.**

---

## mymind

Strong competitor for:

- saving things
- automatic categorization
- bookmarks
- images
- visual memory
- privacy

Current pricing is $7.99/month for Student of Life and $12.99/month for Mastermind.

Official source:
https://access.mymind.com/

Key difference:

**mymind is primarily a visual knowledge-saving environment; your product is an operating layer that can perform actions.**

---

## Readwise Reader

Strong competitor for:

- articles
- PDFs
- highlights
- reading
- browser saving
- resurfacing information

Reader can save articles through its browser extension and keyboard shortcut.

Official source:
https://readwise.io/read

Key difference:

**Readwise is optimized around reading and highlighting; your system should treat articles as only one type of information.**

---

## Capacities

Strong competitor for:

- structured personal knowledge
- objects
- relationships
- collections
- notes

Official source:
https://capacities.io/

Key difference:

**Capacities is a knowledge workspace; your product is designed around zero-friction commands.**

---

## Recaller / Recall-style products

Relevant competitor for:

- saving web information
- knowledge retrieval
- summaries
- learning

Key difference:

Your product should go beyond knowledge capture into **computer actions and personal command execution**.

---

## RemNote

Important competitor for the future learning layer.

It combines:

- notes
- flashcards
- AI-generated cards
- quizzes
- linked knowledge
- spaced repetition
- exam scheduling

It supports AI card generation and spaced repetition, including FSRS. 

Official source:
https://www.remnote.com/

Key difference:

**Your learning layer starts from the same information-capture system rather than requiring the user to move into a separate note-taking workflow.**

---

## Raycast

This is perhaps the most important *interaction-design* competitor.

Raycast already provides:

- global command launcher
- hotkeys
- file search
- quicklinks
- AI commands
- AI extensions
- selected-text access
- browser context
- file-management tools
- memory

It is available on Windows as well as macOS.

Official sources:
https://www.raycast.com/
https://www.raycast.com/windows

Key difference:

Raycast is a **general computer command launcher**.

Your product should specialize in:

> **memory + knowledge + capture + context + personal organization**

Raycast is therefore more of a reference model for the interaction layer than a direct replacement.

---

## Alfred

Important interaction competitor on macOS.

It provides:

- global workflows
- file search
- hotkeys
- snippets
- clipboard functionality
- scripts
- custom actions

Official source:
https://www.alfredapp.com/

Key difference:

Again, Alfred demonstrates the power of a **global command interface**, while your product focuses on personal memory and context.

---

## Screenpipe

Important conceptual competitor.

It focuses on passive computer memory and local capture.

Key difference:

**Screenpipe remembers what the computer saw/heard.**

Your product should primarily remember **what the user explicitly chose to preserve**.

---

# 30. Competitive positioning

The competitive map can be understood like this:

```text
                     MORE AUTOMATIC
                           ↑
                           │
             Screenpipe    │    Passive AI memory
                           │
                           │
       Raycast             │       Mem
                           │
COMMAND ───────────────────┼────────────────── KNOWLEDGE
                           │
       Alfred              │       mymind
                           │
                           │       Readwise
                           │
                           │       Capacities
                           │
                           ↓
                     MORE MANUAL
```

Your desired position:

> **Very low-friction command interface + deliberate memory + automatic organization.**

---

# 31. Product boundaries

The product should NOT attempt to become:

- a full Notion clone
- a full calendar application
- a full project-management system
- a social network
- a passive surveillance system
- a general-purpose autonomous computer agent
- a complete photo-management platform
- a complete learning-management system

Those can be integrations or future modules.

The core remains:

> **Capture → Remember → Retrieve → Act**

---

# 32. Three layers of functionality

## Layer A — Memory

- save
- search
- organize
- connect
- retrieve
- reopen

## Layer B — Action

- create tasks
- reminders
- files
- folders
- downloads
- move/rename
- open things
- research

## Layer C — Intelligence

- understand commands
- classify
- retrieve
- plan
- personalize
- learn user preferences
- local models
- learning/review

This keeps the product understandable.

---

# 33. Local-first architecture

```text
Windows / macOS Desktop Client
            │
            ▼
       Local Core API
            │
     ┌──────┼──────┐
     │      │      │
 PostgreSQL Redis  Model Runtime
 + pgvector         │
     │          ┌──┴────────────┐
     │          │               │
     │       whisper.cpp      Local LLM
     │                          │
     │                     Fish Speech
     │
     └────────── LangGraph ─────┘
```

All core functions should remain available offline where technically possible.

---

# 34. Cloud architecture

Cloud mode should use the same interfaces.

```text
Desktop Client
      ↓
Authenticated API
      ↓
FastAPI
      ↓
LangGraph
      ↓
PostgreSQL + pgvector
      ↓
Cloud model providers
```

Potential cloud services:

- AWS
- object storage
- managed PostgreSQL
- remote model APIs
- remote embeddings
- optional remote speech

The cloud should be a deployment/provider option, not a hard architectural dependency.

---

# 35. Desktop architecture

## Windows first

Recommended:

**Tauri + React/TypeScript**

Responsibilities:

- system tray
- global shortcut
- overlay
- notifications
- permissions
- clipboard
- context acquisition
- communication with local Python core

## macOS

Potential:

**SwiftUI**

Responsibilities:

- native macOS interface
- global shortcut
- accessibility integration
- clipboard
- notifications
- native permissions

Both communicate with the same core API.

---

# 36. Core Python service

Suggested modules:

```text
core/
├── api/
├── agent/
│   ├── graph/
│   ├── state/
│   └── tools/
├── memory/
├── retrieval/
├── embeddings/
├── speech/
├── planning/
├── files/
├── research/
├── learning/
├── models/
├── storage/
└── events/
```

---

# 37. Suggested repository structure

```text
personal-memory-os/
│
├── apps/
│   ├── windows/
│   ├── macos/
│   └── web/
│
├── core/
│   ├── api/
│   ├── agent/
│   ├── memory/
│   ├── retrieval/
│   ├── speech/
│   ├── files/
│   ├── planning/
│   ├── learning/
│   ├── models/
│   └── storage/
│
├── infrastructure/
│   ├── docker/
│   ├── postgres/
│   ├── redis/
│   └── deployment/
│
├── models/
│   ├── embedding/
│   ├── stt/
│   ├── llm/
│   └── tts/
│
├── experiments/
│   ├── rag/
│   ├── langgraph/
│   ├── embeddings/
│   ├── lora/
│   └── local_models/
│
└── docs/
```

---

# 38. Initial implementation target

The first complete product slice should be:

```text
Global shortcut
    ↓
Voice
    ↓
whisper.cpp
    ↓
Command parser
    ↓
Current context
    ↓
LangGraph
    ↓
PostgreSQL
    ↓
pgvector
    ↓
Result
    ↓
Tiny overlay
```

It should support:

1. Save current context.
2. Search memory.
3. Open saved source.
4. Assign/move collections.
5. Create a reminder.
6. Create a task.
7. Browse knowledge.
8. Work offline.

This is the core system.

---

# 39. Advanced implementation targets

After the core works:

### A. Personal routing model

Train/fine-tune a small model using real commands.

### B. Research agent

Search web → evaluate → save selected sources.

### C. File agent

Search/download/create/move/rename files.

### D. Photo memory

Search photos using metadata + visual embeddings.

### E. Planning agent

Tasks + calendar + memory.

### F. Learning mode

Knowledge → quizzes → spaced repetition.

### G. Cross-platform

Windows → macOS → Linux → mobile.

---

# 40. Key technical concepts to master

For the project and CV:

### Backend

- FastAPI
- Pydantic
- PostgreSQL
- SQL
- Redis
- Docker

### AI

- LLM inference
- structured output
- tool calling
- embeddings
- vector search
- hybrid retrieval
- reranking
- RAG
- agent architecture
- LangChain
- LangGraph
- evaluation

### Local AI

- llama.cpp
- Ollama
- whisper.cpp
- local embeddings
- quantization
- GPU inference
- latency optimization

### Fine tuning

- dataset construction
- LoRA
- PEFT
- evaluation
- model serving

### Desktop

- Tauri
- global shortcuts
- system tray
- IPC
- OS permissions
- Windows APIs
- macOS accessibility APIs

---

# 41. The fundamental architecture interview explanation

If asked:

> “What did you build?”

Answer:

> “I built a local-first personal memory system. A desktop client captures context and provides a global voice interface. A Python backend exposes the core functionality through FastAPI. PostgreSQL stores structured knowledge and relationships, while pgvector provides semantic retrieval. LangGraph orchestrates stateful workflows and tool calls. Local speech recognition and LLMs allow the system to work offline, while cloud providers can be swapped in through provider interfaces. I also experimented with fine-tuning a small model on the user's own command history for personalized intent routing.”

That demonstrates architecture rather than buzzwords.

---

# 42. Product evolution

The long-term product can evolve through:

```text
V1
Personal Memory

    ↓

V2
Personal Command Center

    ↓

V3
Personal Computer Assistant

    ↓

V4
Personal Learning Layer

    ↓

V5
Personalized Local AI
```

The same underlying memory system powers all of them.

---

# 43. The deeper product insight

The most valuable thing is not the AI model.

It is the **interaction model**:

> The user should never have to think about where information belongs before saving it.

The system should make the cost of capture almost zero.

That solves the behavioral problem behind:

> “I have 300 tabs.”

> “I know I read this somewhere.”

> “I saved those photos somewhere.”

> “I learned this but forgot it.”

> “I know there was an article about this.”

The system becomes the place where those things go.

---

# 44. Final product definition

**Personal Memory OS** is a local-first desktop assistant that lets users capture information, organize it, retrieve it, and perform actions through a global voice shortcut.

It combines:

- personal knowledge management
- semantic retrieval
- desktop context
- voice interaction
- lightweight agent workflows
- file operations
- task/reminder management
- optional web research
- optional learning/review
- local AI

without requiring the user to manually maintain a complex knowledge system.

The core UX is:

> **Shortcut → speak → done.**

