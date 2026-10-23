import logbox


def test_version() -> None:
    assert logbox.__version__.count(".") == 2
