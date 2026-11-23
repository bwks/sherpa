"""pytest options of the Talos lab tests."""

from pathlib import Path


def pytest_addoption(parser):
    parser.addoption("--keep-failed", action="store_true",
                     help="Leave the lab running for debugging when a case fails")
    parser.addoption("--run-root", type=Path, default=None,
                     help="The run's directory, which the runner gives every lab it runs at once "
                          "(default: a new one under .tmp/lab-tests)")
