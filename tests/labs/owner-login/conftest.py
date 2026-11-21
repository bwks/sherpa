"""pytest options of the owner login lab test."""


def pytest_addoption(parser):
    parser.addoption("--keep-failed", action="store_true",
                     help="Leave the lab and the accounts for debugging when a case fails")
