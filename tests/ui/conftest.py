import pytest
from PySide6.QtWidgets import QApplication

@pytest.fixture(autouse=True)
def clear_test_clipboard(qapp):
    yield
    QApplication.clipboard().clear()
