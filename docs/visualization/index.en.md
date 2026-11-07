---
title: Visualization
nav_order: 11
has_children: true
source_commit: 2985da44
---

# Visualization

The `maharit-viz` crate provides visualization and export of graph data.

## Features

| Feature | Crate module | Description |
|-----|-----------------|------|
| [Web app](web-app.md) | `maharit_viz::web` (binary `maharit-viz`) | An HTTP server for running queries and viewing results as tables and graphs in the browser |
| [DOT / SVG export](export.md) | `maharit_viz::dot`, `maharit_viz::svg` | Export to the Graphviz DOT format and to SVG files |
| [ASCII visualization](ascii.md) | `maharit_viz::ascii` | Human-readable text visualization in the terminal (DFS / BFS) |
