"""pytest options of the P2p links lab test."""


def pytest_addoption(parser):
    parser.addoption("--keep-failed", action="store_true",
                     help="Leave the lab running for debugging when a case fails")
    parser.addoption("--sherpad-restart", metavar="COMMAND", default=None,
                     help="Also check links after running COMMAND, which restarts sherpad "
                          "(e.g. 'sudo systemctl restart sherpad')")
