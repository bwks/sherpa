"""pytest options of the tailnet lab test."""

import os


def pytest_addoption(parser):
    parser.addoption("--admin-user", default=os.environ.get("SHERPA_LAB_TEST_ADMIN_USER"),
                     help="An admin who does not own the labs, such as the built-in admin, who runs "
                          "`sherpa server clean` (default: $SHERPA_LAB_TEST_ADMIN_USER). "
                          "Its password is read from $SHERPA_LAB_TEST_ADMIN_PASSWORD")
    parser.addoption("--keep-failed", action="store_true",
                     help="Leave the labs and the probe running for debugging when a case fails")
