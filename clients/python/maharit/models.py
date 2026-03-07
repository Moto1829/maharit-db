"""MaharitDB domain model classes for Node and Edge."""

from typing import Any, Dict, List, Optional


class Node:
    """Represents a graph node returned from a query.

    Attributes:
        id: Unique node ID assigned by the database.
        labels: List of labels attached to this node.
        properties: Key-value properties stored on the node.

    Example::

        result = client.query("MATCH (n:Person) RETURN n")
        for row in result:
            node = Node.from_dict(row["n"])
            print(node.id, node.labels, node["name"])
    """

    def __init__(
        self,
        id: int,
        labels: Optional[List[str]] = None,
        properties: Optional[Dict[str, Any]] = None,
    ) -> None:
        self.id = id
        self.labels: List[str] = labels if labels is not None else []
        self.properties: Dict[str, Any] = properties if properties is not None else {}

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> "Node":
        """Construct a Node from a result dict.

        The dict is expected to have the shape returned by the server::

            {"id": 1, "labels": ["Person"], "properties": {"name": "Alice"}}

        Args:
            data: Raw node dict from a query result row.

        Returns:
            A :class:`Node` instance.
        """
        return cls(
            id=data["id"],
            labels=list(data.get("labels", [])),
            properties=dict(data.get("properties", {})),
        )

    def get(self, key: str, default: Any = None) -> Any:
        """Return the property value for *key*, or *default* if not present."""
        return self.properties.get(key, default)

    def __getitem__(self, key: str) -> Any:
        return self.properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.properties

    def __eq__(self, other: object) -> bool:
        if not isinstance(other, Node):
            return NotImplemented
        return self.id == other.id

    def __hash__(self) -> int:
        return hash(self.id)

    def __repr__(self) -> str:
        labels_str = ":".join(self.labels)
        return f"Node(id={self.id}, :{labels_str}, {self.properties})"


class Edge:
    """Represents a graph edge (relationship) returned from a query.

    Attributes:
        id: Unique edge ID assigned by the database.
        type: Relationship type label (e.g. ``"KNOWS"``).
        start_node_id: ID of the source node.
        end_node_id: ID of the target node.
        properties: Key-value properties stored on the edge.

    Example::

        result = client.query("MATCH ()-[r:KNOWS]->() RETURN r")
        for row in result:
            edge = Edge.from_dict(row["r"])
            print(edge.type, edge.start_node_id, edge.end_node_id)
    """

    def __init__(
        self,
        id: int,
        type: str,
        start_node_id: int,
        end_node_id: int,
        properties: Optional[Dict[str, Any]] = None,
    ) -> None:
        self.id = id
        self.type = type
        self.start_node_id = start_node_id
        self.end_node_id = end_node_id
        self.properties: Dict[str, Any] = properties if properties is not None else {}

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> "Edge":
        """Construct an Edge from a result dict.

        The dict is expected to have the shape returned by the server::

            {
                "id": 5,
                "type": "KNOWS",
                "startNodeId": 1,
                "endNodeId": 2,
                "properties": {"since": 2020}
            }

        Args:
            data: Raw edge dict from a query result row.

        Returns:
            An :class:`Edge` instance.
        """
        return cls(
            id=data["id"],
            type=data["type"],
            start_node_id=data["startNodeId"],
            end_node_id=data["endNodeId"],
            properties=dict(data.get("properties", {})),
        )

    def get(self, key: str, default: Any = None) -> Any:
        """Return the property value for *key*, or *default* if not present."""
        return self.properties.get(key, default)

    def __getitem__(self, key: str) -> Any:
        return self.properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.properties

    def __eq__(self, other: object) -> bool:
        if not isinstance(other, Edge):
            return NotImplemented
        return self.id == other.id

    def __hash__(self) -> int:
        return hash(self.id)

    def __repr__(self) -> str:
        return (
            f"Edge(id={self.id}, type={self.type!r}, "
            f"{self.start_node_id}->{self.end_node_id}, {self.properties})"
        )
