"""One-time codes: the properties that keep six digits safe."""

import pytest

from app import codes


def test_a_code_is_six_digits_from_the_os_random_source():
    for _ in range(50):
        c = codes.generate()
        assert len(c) == codes.LENGTH
        assert c.isdigit()


def test_two_codes_are_not_the_same():
    """Not proof of randomness, but it catches a constant or a bad seed."""
    assert len({codes.generate() for _ in range(200)}) > 150


def test_a_code_matches_only_its_own_hash():
    c = codes.generate()
    h = codes.hash_code(c)
    assert codes.matches(c, h)
    assert not codes.matches("000000", h) or c == "000000"


def test_the_code_is_never_stored_in_the_clear():
    c = "123456"
    assert c not in codes.hash_code(c)


@pytest.mark.parametrize(
    "raw,expected",
    [("  A@B.com ", "a@b.com"), ("someone@Example.COM", "someone@example.com")],
)
def test_addresses_are_normalised_so_one_person_is_one_account(raw, expected):
    assert codes.normalise(raw) == expected


def test_normalising_leaves_plus_addressing_alone():
    """Some providers treat these as equivalent and some do not. Merging them
    would join addresses their owner considers separate."""
    assert codes.normalise("a+work@b.com") == "a+work@b.com"
    assert codes.normalise("first.last@b.com") == "first.last@b.com"


@pytest.mark.parametrize("good", ["a@b.com", "first.last@sub.example.co.uk", "x+y@z.io"])
def test_plausible_addresses_pass(good):
    assert codes.looks_like_an_address(good)


@pytest.mark.parametrize(
    "bad",
    ["", "nope", "a@b", "a@@b.com", "@b.com", "a@", "a@.com", "a@b.", "a" * 250 + "@b.com"],
)
def test_implausible_addresses_are_caught_before_a_send_is_spent(bad):
    assert not codes.looks_like_an_address(bad)
