"""pytest options of the console lab test."""

import os


def pytest_addoption(parser):
    parser.addoption("--admin-user", default=os.environ.get("SHERPA_LAB_TEST_ADMIN_USER"),
                     help="An admin who does not own the lab, such as the built-in admin; it creates and deletes the "
                          "run's other user (default: $SHERPA_LAB_TEST_ADMIN_USER). "
                          "Its password is read from $SHERPA_LAB_TEST_ADMIN_PASSWORD")
    parser.addoption("--keep-failed", action="store_true",
                     help="Leave the lab running for debugging when a case fails")
    parser.addoption("--no-container", action="store_true",
                     help="Leave the container out of the lab, for a server without its image; "
                          "the container cases are skipped")
