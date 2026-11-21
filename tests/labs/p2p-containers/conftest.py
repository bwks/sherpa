"""pytest options of the P2p containers lab test."""


def pytest_addoption(parser):
    parser.addoption("--keep-failed", action="store_true",
                     help="Leave the lab running for debugging when a case fails")
