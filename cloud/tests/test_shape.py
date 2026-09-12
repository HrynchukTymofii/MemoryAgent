"""What the shaper does with what the model hands back.

The model itself is not exercised here and deliberately so: `clean` is the half
that decides whether a 0.6B model's answer is allowed to reach the user's
document, and it is worth more than a test that a particular checkpoint happened
to produce a particular list on a particular day. These run with no model, no
llama-cpp-python, and no network.
"""

from app.shape import Shaper, Unavailable, clean

SPOKEN = (
    "so I need to do a few things tomorrow first edit the videos second of all "
    "create four new scripts third of all find images and finally record four "
    "new videos"
)


# What a good reply looks like: about as many words as were spoken, which is
# exactly what the length guard in `clean` is checking for.
SHAPED = (
    "I need to do a few things tomorrow:\n"
    "\n"
    "1. Edit the videos.\n"
    "2. Create four new scripts.\n"
    "3. Find images.\n"
    "4. Record four new videos."
)


def test_a_clean_reply_is_passed_through():
    assert clean(SHAPED, SPOKEN) == SHAPED


def test_thinking_blocks_never_reach_the_document():
    """`/no_think` suppresses the content of the block, not the tags."""
    raw = "<think>The user is listing tasks.</think>\n\n" + SHAPED
    assert clean(raw, SPOKEN) == SHAPED


def test_an_unclosed_thinking_block_is_left_alone_rather_than_half_stripped():
    # Truncation mid-thought: there is no closing tag to cut at, and guessing
    # where the thought ended would be worse than the length check catching it.
    raw = "<think>The user is listing tasks and I should"
    assert clean(raw, SPOKEN) == SPOKEN


def test_a_code_fence_is_unwrapped():
    raw = "```markdown\n" + SHAPED + "\n```"
    assert clean(raw, SPOKEN) == SHAPED


def test_an_empty_reply_falls_back_to_the_spoken_words():
    assert clean("", SPOKEN) == SPOKEN
    assert clean("   \n  ", SPOKEN) == SPOKEN
    # And a reply that was nothing but thinking.
    assert clean("<think>hmm</think>", SPOKEN) == SPOKEN


def test_a_model_that_answers_instead_of_transcribing_is_discarded():
    """The failure that reads like a success.

    A small model asked to format a to-do list sometimes helps with it instead.
    That comes back well-formed and confident, and typing it into somebody's
    document would put words there they never said.
    """
    answered = (
        "Great plan! Here is how I would approach tomorrow. Start with the "
        "scripts, since editing without a script is wasted effort, and batch the "
        "image search so you are not switching context. You could also consider "
        "recording two videos in one session to save setup time, and scheduling "
        "the post for the morning when engagement is highest. Let me know if you "
        "would like help with any of these steps and I can go into more detail."
    )
    assert clean(answered, SPOKEN) == SPOKEN


def test_a_model_that_summarises_is_discarded():
    assert clean("Some video tasks.", SPOKEN) == SPOKEN


def test_a_short_utterance_is_not_length_checked():
    """Below eight words the ratio says nothing.

    "remind me tomorrow" formatted into "Remind me tomorrow." is a 1:1 rewrite,
    but "okay" to "Okay." is 1:1 on a sample of one word — and any real
    formatting of a short phrase can legitimately double or halve its length.
    """
    assert clean("Remind me tomorrow.", "remind me tomorrow") == "Remind me tomorrow."
    assert clean("Okay.", "okay") == "Okay."


def test_a_missing_model_file_says_what_to_run():
    shaper = Shaper(model_path="does/not/exist.gguf")
    assert not shaper.ready()
    try:
        shaper.load()
    except Unavailable as e:
        assert "fetch-models" in str(e)
    else:  # pragma: no cover
        raise AssertionError("a missing model must not load")
