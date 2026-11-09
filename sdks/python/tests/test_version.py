import trailscope


def test_version() -> None:
    assert trailscope.__version__.count(".") == 2
