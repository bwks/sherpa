"""pytest options of the owner login lab test."""

import os


def pytest_addoption(parser):
    parser.addoption("--admin-user", default=os.environ.get("SHERPA_LAB_TEST_ADMIN_USER"),
                     help="An admin account other than `admin` that creates and deletes the throwaway owner "
                          "(default: $SHERPA_LAB_TEST_ADMIN_USER). Its password is read from "
                          "$SHERPA_LAB_TEST_ADMIN_PASSWORD")
    parser.addoption("--keep-failed", action="store_true",
                     help="Leave the lab and the owner for debugging when a case fails")
