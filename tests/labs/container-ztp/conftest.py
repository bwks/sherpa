"""pytest options of the container ZTP lab test."""


def pytest_addoption(parser):
    parser.addoption("--keep-failed", action="store_true",
                     help="Leave the lab running for debugging when a case fails")
    parser.addoption("--models", default="",
                     help="Comma-separated models to run, for a server without every image "
                          "(default: every node in the manifest)")
