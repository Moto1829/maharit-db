"""Tests for Node and Edge model classes."""

import pytest

from maharit.models import Edge, Node


class TestNode:
    def test_basic_construction(self):
        node = Node(id=1, labels=["Person"], properties={"name": "Alice", "age": 30})
        assert node.id == 1
        assert node.labels == ["Person"]
        assert node.properties == {"name": "Alice", "age": 30}

    def test_defaults(self):
        node = Node(id=42)
        assert node.labels == []
        assert node.properties == {}

    def test_from_dict(self):
        data = {
            "id": 7,
            "labels": ["Person", "Employee"],
            "properties": {"name": "Bob", "age": 25},
        }
        node = Node.from_dict(data)
        assert node.id == 7
        assert node.labels == ["Person", "Employee"]
        assert node.properties == {"name": "Bob", "age": 25}

    def test_from_dict_missing_optional_fields(self):
        node = Node.from_dict({"id": 3})
        assert node.labels == []
        assert node.properties == {}

    def test_getitem(self):
        node = Node(id=1, properties={"name": "Alice"})
        assert node["name"] == "Alice"

    def test_getitem_missing_raises(self):
        node = Node(id=1)
        with pytest.raises(KeyError):
            _ = node["missing"]

    def test_get_with_default(self):
        node = Node(id=1, properties={"name": "Alice"})
        assert node.get("name") == "Alice"
        assert node.get("missing") is None
        assert node.get("missing", "default") == "default"

    def test_contains(self):
        node = Node(id=1, properties={"name": "Alice"})
        assert "name" in node
        assert "age" not in node

    def test_equality(self):
        n1 = Node(id=1, labels=["A"], properties={"x": 1})
        n2 = Node(id=1, labels=["B"], properties={"x": 2})
        n3 = Node(id=2)
        assert n1 == n2  # same id
        assert n1 != n3

    def test_hash(self):
        n1 = Node(id=5)
        n2 = Node(id=5)
        assert hash(n1) == hash(n2)
        s = {n1, n2}
        assert len(s) == 1

    def test_repr(self):
        node = Node(id=1, labels=["Person"], properties={"name": "Alice"})
        r = repr(node)
        assert "1" in r
        assert "Person" in r

    def test_from_dict_isolates_properties(self):
        """Mutating returned Node.properties should not affect the original dict."""
        data = {"id": 1, "labels": ["A"], "properties": {"x": 1}}
        node = Node.from_dict(data)
        node.properties["y"] = 2
        assert "y" not in data["properties"]


class TestEdge:
    def test_basic_construction(self):
        edge = Edge(id=10, type="KNOWS", start_node_id=1, end_node_id=2, properties={"since": 2020})
        assert edge.id == 10
        assert edge.type == "KNOWS"
        assert edge.start_node_id == 1
        assert edge.end_node_id == 2
        assert edge.properties == {"since": 2020}

    def test_defaults(self):
        edge = Edge(id=1, type="REL", start_node_id=0, end_node_id=1)
        assert edge.properties == {}

    def test_from_dict(self):
        data = {
            "id": 5,
            "type": "KNOWS",
            "startNodeId": 1,
            "endNodeId": 2,
            "properties": {"since": 2021},
        }
        edge = Edge.from_dict(data)
        assert edge.id == 5
        assert edge.type == "KNOWS"
        assert edge.start_node_id == 1
        assert edge.end_node_id == 2
        assert edge.properties == {"since": 2021}

    def test_from_dict_no_properties(self):
        data = {"id": 1, "type": "LIKES", "startNodeId": 3, "endNodeId": 4}
        edge = Edge.from_dict(data)
        assert edge.properties == {}

    def test_getitem(self):
        edge = Edge(id=1, type="R", start_node_id=0, end_node_id=1, properties={"w": 0.5})
        assert edge["w"] == 0.5

    def test_get_with_default(self):
        edge = Edge(id=1, type="R", start_node_id=0, end_node_id=1)
        assert edge.get("missing", 99) == 99

    def test_contains(self):
        edge = Edge(id=1, type="R", start_node_id=0, end_node_id=1, properties={"w": 1})
        assert "w" in edge
        assert "x" not in edge

    def test_equality(self):
        e1 = Edge(id=3, type="A", start_node_id=0, end_node_id=1)
        e2 = Edge(id=3, type="B", start_node_id=9, end_node_id=9)
        e3 = Edge(id=4, type="A", start_node_id=0, end_node_id=1)
        assert e1 == e2
        assert e1 != e3

    def test_hash(self):
        e1 = Edge(id=7, type="X", start_node_id=0, end_node_id=1)
        e2 = Edge(id=7, type="Y", start_node_id=2, end_node_id=3)
        assert hash(e1) == hash(e2)
        s = {e1, e2}
        assert len(s) == 1

    def test_repr(self):
        edge = Edge(id=5, type="KNOWS", start_node_id=1, end_node_id=2)
        r = repr(edge)
        assert "KNOWS" in r
        assert "1" in r
        assert "2" in r

    def test_from_dict_isolates_properties(self):
        data = {"id": 1, "type": "R", "startNodeId": 0, "endNodeId": 1, "properties": {"x": 1}}
        edge = Edge.from_dict(data)
        edge.properties["y"] = 2
        assert "y" not in data["properties"]
