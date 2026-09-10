"""Referrals: the rules that stop this being a way to mint free months.

Everything here is the pure half — codes, windows, arithmetic. The constraints
that do the real work (one referral per account, one payout per side) live in
`003_referrals.sql` and are exercised against a database rather than here; what
these cover is the logic the endpoints branch on before they get there.
"""

from datetime import datetime, timedelta, timezone

from app import referrals


def test_a_code_carries_the_name_and_a_random_tail():
    code = referrals.generate("Tymofii Hrynchuk", "tim@example.com")
    assert code.startswith("TYMOFIIHRY")
    assert len(code) == referrals.NAME_MAX + referrals.SUFFIX


def test_the_random_half_uses_only_unambiguous_characters():
    """I/1, O/0 and S/5 cost a support message every time they are read aloud."""
    for _ in range(200):
        code = referrals.generate("Sonia Iossifova", None)
        assert set(code[-referrals.SUFFIX :]) <= set(referrals.ALPHABET)


def test_the_name_half_keeps_the_letters_the_name_has():
    """Stripping I, O and S out of a name gives back something nobody owns."""
    assert referrals.generate("Tymofii", None).startswith("TYMOFII")
    assert referrals.generate("Sonia", None).startswith("SONIA")


def test_two_codes_for_the_same_person_differ():
    made = {referrals.generate("Alex", None) for _ in range(200)}
    assert len(made) > 150


def test_a_person_with_no_usable_name_still_gets_a_code():
    """An account whose token carried neither field must still be able to refer."""
    code = referrals.generate(None, None)
    assert len(code) == referrals.SUFFIX
    assert referrals.looks_like_a_code(code)


def test_a_name_that_is_not_latin_leaves_the_tail_alone():
    code = referrals.generate("Тимофій", None)
    assert len(code) == referrals.SUFFIX


def test_a_pasted_code_normalises_to_the_one_that_was_sent():
    sent = "TYMOFII28QK"
    for pasted in (" tymofii28qk ", "TYMOFII28QK.", "<TYMOFII28QK>", "tymofii-28qk"):
        assert referrals.normalise(pasted) == sent


def test_an_invite_link_survives_a_trailing_slash():
    assert (
        referrals.invite_link("https://api.example.com/", "ABCD1234")
        == "https://api.example.com/r/ABCD1234"
    )


def test_qualification_is_the_threshold_and_not_a_sign_up():
    assert not referrals.has_qualified(0, 2_000)
    assert not referrals.has_qualified(1_999, 2_000)
    assert referrals.has_qualified(2_000, 2_000)


def test_a_code_may_only_be_applied_by_a_new_account():
    fresh = datetime.now(timezone.utc) - timedelta(days=3)
    old = datetime.now(timezone.utc) - timedelta(days=45)
    assert referrals.may_apply(account_created_at=fresh, window_days=30)
    assert not referrals.may_apply(account_created_at=old, window_days=30)


def test_a_naive_timestamp_is_read_as_utc_rather_than_crashing():
    """Postgres can hand back a naive datetime through a misconfigured driver.

    Treating that as UTC is right; letting it raise inside a comparison would
    take out the whole endpoint over a timezone.
    """
    naive = (datetime.now(timezone.utc) - timedelta(days=1)).replace(tzinfo=None)
    assert referrals.may_apply(account_created_at=naive, window_days=30)


def test_two_rewards_stack_rather_than_overwrite():
    """A month granted to somebody with three weeks left leaves seven."""
    first = referrals.extend(None, 1)
    second = referrals.extend(first, 1)
    assert (second - first).days == 30


def test_a_reward_to_a_lapsed_account_runs_from_today():
    """Not from an expiry that passed in March — that would grant nothing."""
    lapsed = datetime.now(timezone.utc) - timedelta(days=200)
    granted = referrals.extend(lapsed, 1)
    assert granted > datetime.now(timezone.utc) + timedelta(days=29)


def test_pro_is_a_date_in_the_future_and_nothing_else():
    assert not referrals.is_pro(None)
    assert not referrals.is_pro(datetime.now(timezone.utc) - timedelta(seconds=1))
    assert referrals.is_pro(datetime.now(timezone.utc) + timedelta(days=1))


def test_a_code_has_to_be_long_enough_to_be_one():
    assert not referrals.looks_like_a_code("AB")
    assert referrals.looks_like_a_code("ABCD")
    assert not referrals.looks_like_a_code("A" * 40)
