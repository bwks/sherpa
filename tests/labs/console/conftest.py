"""pytest options of the console lab test."""

import os


def pytest_addoption(parser):
    parser.addoption("--other-user", default=os.environ.get("SHERPA_LAB_TEST_OTHER_USER"),
                     help="A normal user who does not own the lab (default: $SHERPA_LAB_TEST_OTHER_USER). "
                          "Its password is read from $SHERPA_LAB_TEST_OTHER_PASSWORD")
    parser.addoption("--admin-user", default=os.environ.get("SHERPA_LAB_TEST_ADMIN_USER"),
                     help="An admin who does not own the lab (default: $SHERPA_LAB_TEST_ADMIN_USER). "
                          "Its password is read from $SHERPA_LAB_TEST_ADMIN_PASSWORD")
    parser.addoption("--keep-failed", action="store_true",
                     help="Leave the lab running for debugging when a case fails")
    parser.addoption("--no-container", action="store_true",
                     help="Leave the container out of the lab, for a server without its image; "
                          "the container cases are skipped")
