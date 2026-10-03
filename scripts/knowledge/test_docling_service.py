"""Adapter contract tests; no Docling models or downloads are needed."""

import sys
import tempfile
import unittest
from pathlib import Path
from types import ModuleType, SimpleNamespace
from unittest.mock import patch

import docling_service


class Box:
    def __init__(self, left=10, top=20, right=90, bottom=40):
        self.l, self.t, self.r, self.b = left, top, right, bottom

    def to_top_left_origin(self, *, page_height):
        return self


def provenance(page, box=None):
    # Table char spans do not index the exported Markdown.
    return SimpleNamespace(page_no=page, bbox=box or Box(), charspan=(0, 0))


class TextItem:
    def __init__(self, text, prov):
        self.text, self.prov = text, prov
        self.label = SimpleNamespace(value="text")


class TableItem(TextItem):
    def export_to_markdown(self, *, doc):
        return self.text


class AdapterTests(unittest.TestCase):
    def convert(self, item):
        module = ModuleType("docling_core.types.doc")
        module.TableItem, module.TextItem = TableItem, TextItem
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "source.pdf"
            path.write_bytes(b"%PDF fixture")
            doc = SimpleNamespace(
                pages={page: SimpleNamespace(size=SimpleNamespace(width=100, height=200))
                       for page in (1, 2)},
                iterate_items=lambda: iter([(item, 0)]),
                save_as_json=lambda destination: destination.write_text("{}"),
            )
            converter = SimpleNamespace(convert=lambda path: SimpleNamespace(
                status=SimpleNamespace(value="success"), document=doc))
            with patch.dict(sys.modules, {"docling_core.types.doc": module}), \
                    patch.object(docling_service.importlib.metadata, "version", return_value="test"):
                return docling_service.convert_pdf(path, Path(folder) / "cache", converter, ["en"])

    def test_multi_page_table_is_retained_once_without_a_false_page(self):
        text = "| item | amount |\n| --- | --- |\n| first-page | 10 |\n| later-page-only | 20 |"
        result = self.convert(TableItem(text, [provenance(1), provenance(2)]))
        self.assertEqual(len(result["blocks"]), 1)
        block = result["blocks"][0]
        self.assertEqual(block["text"], text)
        self.assertIsNone(block.get("page"))
        self.assertIsNone(block.get("bbox"))
        self.assertEqual(block["sourcePages"], [1, 2])
        self.assertTrue(block["section"])
        self.assertEqual(result["protocol"], 2)
        self.assertTrue(result["parserVersion"].startswith("nexa-adapter-2:"))

    def test_single_page_table_keeps_normalized_location(self):
        result = self.convert(TableItem("| one | 10 |", [provenance(2)]))
        self.assertEqual(len(result["blocks"]), 1)
        self.assertEqual(result["blocks"][0]["page"], 2)
        self.assertEqual(result["blocks"][0]["bbox"], [0.1, 0.1, 0.9, 0.2])

    def test_same_page_fragments_are_one_block_with_a_covering_box(self):
        result = self.convert(TableItem("both fragments", [
            provenance(1, Box(10, 20, 50, 40)), provenance(1, Box(20, 60, 90, 100))]))
        self.assertEqual(len(result["blocks"]), 1)
        self.assertEqual(result["blocks"][0]["bbox"], [0.1, 0.1, 0.9, 0.5])

    def test_multi_page_text_does_not_copy_later_text_to_earlier_page(self):
        result = self.convert(TextItem("first page and later page", [provenance(1), provenance(2)]))
        self.assertEqual(len(result["blocks"]), 1)
        self.assertIsNone(result["blocks"][0].get("page"))
        self.assertEqual(result["blocks"][0]["sourcePages"], [1, 2])


if __name__ == "__main__":
    unittest.main()
