"""Sphinx configuration for maharit Python client docs."""

import os
import sys

# Allow autodoc to import the package
sys.path.insert(0, os.path.abspath(".."))

project = "maharit"
copyright = "2024, maharit-db contributors"
author = "maharit-db contributors"
release = "0.1.0"

extensions = [
    "sphinx.ext.autodoc",
    "sphinx.ext.napoleon",
    "sphinx.ext.viewcode",
    "sphinx.ext.intersphinx",
]

intersphinx_mapping = {
    "python": ("https://docs.python.org/3", None),
}

templates_path = ["_templates"]
exclude_patterns = ["_build"]

html_theme = "alabaster"
html_static_path = ["_static"]

autodoc_member_order = "bysource"
napoleon_google_docstring = True
napoleon_numpy_docstring = False
