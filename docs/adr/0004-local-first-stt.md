# ADR-0004: Local-first STT with an always-open microphone

**Status:** Accepted · **Date:** 2026-09-06 · **Implements:** spec section 18

## Context

Wispr Flow, the interaction-design reference for this product, transcribes in
the cloud. The obvious inference is that we should too. Spec Principle 4
(local-first) says otherwise. Both cannot be satisfied by default.

## The distinction that settles it

**For Wispr Flow, the transcript is the product.** It types verbatim text into
the user's editor; every word error is a visible defect the user must repair by
hand. Maximising raw accuracy is therefore the entire game, and that means the
largest available model, which means cloud. Correct for them.

**For this product, the transcript is an intermediate representation.** It is
consumed by an intent router:

```
"save this to React"  ->  { intent: SAVE, collection: "React" }
```

We need enough signal to fill slots, not verbatim perfection. That is a
materially easier task, and a much smaller model clears it.

Local additionally buys a capability cloud APIs largely deny us: **decoder
biasing.** whisper accepts an `initial_prompt`, into which we inject the user's
real collection names, tags and recently seen entities. So "put this in Flari"
transcribes as **Flari**, not "flurry". Proper nouns are simultaneously the
tokens the intent router most depends on and the ones generic cloud models most
often get wrong. This is an advantage of local, not a consolation.

Supporting reasons: a free tier is impossible if every command hits a metered
API, and a Ukraine-to-US round trip costs 100-150 ms before any inference runs.

## Decision

**Default: local.** whisper.cpp, streaming, model chosen by hardware tier
(tiny.en / base.en / small.en / large-v3-turbo). Cloud STT is an **opt-in Pro
upgrade** for accuracy, behind the same `SttProvider` trait.

**The microphone stream is opened at application start and never closed.**
Audio flows continuously into a fixed-size ring buffer. The hotkey does not open
a device — it records a read offset. Opening a WASAPI capture device costs
100-300 ms; paying that on hotkey press clips the first word and is the single
most common way this class of product feels broken.

Transcription is **streaming**: rolling windows are transcribed while the user is
still speaking, so key-release leaves only the ~300 ms tail outstanding.

## Consequences

**Good**

- Zero marginal cost per command, so a free tier is viable.
- Works offline (Principle 4), and no audio leaves the machine by default —
  a real privacy claim, which is a selling point against Wispr Flow.
- Decoder biasing measurably improves exactly the tokens that matter.

**Bad**

- ~200-600 MB resident for the whisper model, permanently. This is a deliberate
  purchase: residency is what buys the sub-second interaction.
- Accuracy on strong accents and long dictation will trail cloud. Mitigated by
  the Pro cloud path, and softened by the fact that we need slots, not prose.
- The always-open microphone must be visibly and honestly communicated: a tray
  indicator, an explicit "audio is discarded unless you press the hotkey"
  guarantee, and a real mute control. Getting this wrong is a trust catastrophe.

**Neutral**

- The ring buffer enables a genuinely useful future feature: retroactive capture
  ("save what I just said"), since the last N seconds are already in memory.
