"""Tests for the maharit_native extension.

Run after `maturin develop`:  python -m unittest discover -s tests
"""

import unittest

from maharit_native import Graph


class GraphTest(unittest.TestCase):
    def test_nodes_edges_and_properties(self):
        g = Graph()
        alice = g.create_node("Person")
        bob = g.create_node_with_labels(["Person", "Employee"])
        g.set_node_property(alice, "name", "Alice")
        g.set_node_property_int(alice, "age", 30)
        g.set_node_property_float(alice, "score", 1.5)
        g.set_node_property_bool(alice, "active", True)
        edge = g.create_edge(alice, bob, "KNOWS")
        g.set_edge_property(edge, "since", "2020")

        self.assertEqual(g.node_count(), 2)
        self.assertEqual(g.edge_count(), 1)
        self.assertEqual(
            g.get_node(alice),
            {
                "id": alice,
                "labels": ["Person"],
                "properties": {"name": "Alice", "age": 30, "score": 1.5, "active": True},
            },
        )
        self.assertIs(g.get_node(alice)["properties"]["active"], True)
        self.assertEqual(sorted(g.get_node(bob)["labels"]), ["Employee", "Person"])
        self.assertEqual(
            g.get_edge(edge),
            {"id": edge, "label": "KNOWS", "from": alice, "to": bob, "properties": {"since": "2020"}},
        )
        self.assertEqual(g.get_outgoing_edges(alice), [edge])
        self.assertEqual(g.get_incoming_edges(bob), [edge])

    def test_missing_ids(self):
        g = Graph()
        self.assertIsNone(g.get_node(42))
        self.assertIsNone(g.get_edge(42))
        with self.assertRaises(ValueError):
            g.create_edge(1, 2, "KNOWS")

    def test_delete(self):
        g = Graph()
        a = g.create_node("A")
        b = g.create_node("B")
        e = g.create_edge(a, b, "R")
        self.assertTrue(g.delete_edge(e))
        self.assertFalse(g.delete_edge(e))
        self.assertTrue(g.delete_node(a))
        self.assertFalse(g.delete_node(a))
        self.assertEqual(g.node_count(), 1)
        self.assertEqual(g.edge_count(), 0)


if __name__ == "__main__":
    unittest.main()
