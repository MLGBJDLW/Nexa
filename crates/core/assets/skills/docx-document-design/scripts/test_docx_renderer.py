#!/usr/bin/env python3
"""Tests for DOCX Spec v2 rendering."""

from __future__ import annotations

import tempfile
import unittest
import zipfile
from pathlib import Path

import docx_renderer


class DocxRendererTests(unittest.TestCase):
    def test_typography_covers_scripts_and_all_generated_roles(self) -> None:
        from docx import Document
        from docx.oxml.ns import qn

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            output = root / 'typography.docx'
            docx_renderer.render_docx({
                'schemaVersion': 2, 'language': 'zh-CN',
                'title': '统一 Title', 'subtitle': '统一 Subtitle',
                'tokens': {'bodyFont': 'Arial', 'headingFont': 'Georgia',
                           'bodyEastAsiaFont': 'Microsoft YaHei', 'headingEastAsiaFont': 'SimHei',
                           'bodySize': 12, 'titleSize': 28},
                'header': {'text': '页眉 Header'}, 'footer': {'text': '页脚 Footer', 'pageNumber': True},
                'blocks': [
                    {'type': 'heading', 'text': '标题 Heading'},
                    {'type': 'paragraph', 'text': '正文 Body [链接 Link](https://example.com)'},
                    {'type': 'bullets', 'items': ['列表 List']},
                    {'type': 'numbered', 'items': ['编号 Number']},
                    {'type': 'table', 'headers': ['项目 Item'], 'rows': [['数据 Data']], 'caption': '说明 Caption'},
                    {'type': 'callout', 'text': '提示 Callout'},
                ],
            }, output, root)
            document = Document(output)
            for name in ('Normal', 'Title', 'Subtitle', 'Heading 1', 'Heading 6',
                         'List Bullet', 'List Number', 'Header', 'Footer', 'Nexa Caption'):
                with self.subTest(style=name):
                    style = document.styles[name]
                    fonts = style.element.rPr.find(qn('w:rFonts'))
                    heading = name == 'Title' or name.startswith('Heading ')
                    self.assertEqual('Georgia' if heading else 'Arial', fonts.get(qn('w:ascii')))
                    self.assertEqual('SimHei' if heading else 'Microsoft YaHei', fonts.get(qn('w:eastAsia')))
                    self.assertEqual('Georgia' if heading else 'Arial', fonts.get(qn('w:cs')))
                    self.assertFalse(any(key.lower().endswith('theme') for key in fonts.attrib))
                    self.assertEqual(style.element.rPr.find(qn('w:sz')).get(qn('w:val')),
                                     style.element.rPr.find(qn('w:szCs')).get(qn('w:val')))
            self.assertEqual('Title', document.paragraphs[0].style.name)
            self.assertEqual(12, document.styles['Normal'].font.size.pt)
            self.assertEqual(28, document.styles['Title'].font.size.pt)

    def test_template_typography_and_page_geometry_remain_authoritative(self) -> None:
        from docx import Document
        from docx.shared import Inches, Pt

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            template = root / 'template.docx'
            document = Document()
            document.styles['Normal'].font.name = 'SimSun'
            document.styles['Normal'].font.size = Pt(13)
            document.styles['Title'].font.name = 'SimHei'
            document.styles['Title'].font.size = Pt(32)
            document.sections[0].left_margin = Inches(1.25)
            document.save(template)
            expected = {name: document.styles[name].element.xml for name in ('Normal', 'Title', 'Heading 1', 'Header', 'Footer')}
            output = root / 'report.docx'
            docx_renderer.render_docx({'schemaVersion': 2, 'template': 'template.docx', 'title': '模板报告',
                                      'blocks': [{'type': 'paragraph', 'text': '正文 Body'}]}, output, root)
            document = Document(output)
            for name, xml in expected.items():
                self.assertEqual(xml, document.styles[name].element.xml)
            self.assertEqual(1.25, document.sections[0].left_margin.inches)

    def test_renders_styles_table_geometry_headers_fields_and_image_alt_text(self) -> None:
        try:
            from PIL import Image
            import docx
        except ImportError:
            self.skipTest("Pillow and python-docx are required")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp).resolve()
            image = root / "chart.png"
            Image.new("RGB", (320, 160), (60, 110, 160)).save(image)
            output = root / "report.docx"
            spec = {
                "schemaVersion": 2,
                "preset": "executive",
                "title": "董事会报告",
                "subtitle": "Board Report",
                "author": "Nexa QA",
                "language": "zh-CN",
                "page": {"marginLeft": 0.8, "marginRight": 0.8},
                "header": {"text": "Confidential"},
                "footer": {"text": "Board", "pageNumber": True},
                "blocks": [
                    {"type": "heading", "level": 1, "text": "执行摘要"},
                    {"type": "paragraph", "text": "See [source](https://example.com/source)."},
                    {
                        "type": "table",
                        "headers": ["Metric", "Value"],
                        "rows": [["Revenue", "100"], ["Margin", "42%"]],
                        "columnWidths": [3.6, 2.2],
                        "repeatHeader": True,
                        "allowRowBreaks": False,
                    },
                    {
                        "type": "image",
                        "path": str(image),
                        "width": 4.0,
                        "altText": "蓝色收入趋势图",
                        "caption": "Figure 1 — Revenue",
                    },
                    {"type": "callout", "kind": "risk", "text": "Risk requires a decision."},
                ],
            }

            result = docx_renderer.render_docx(spec, output, root)
            self.assertEqual(1, result["metrics"]["tables"])
            self.assertEqual(1, result["metrics"]["images"])
            document = docx.Document(output)
            self.assertEqual("Nexa QA", document.core_properties.author)
            self.assertEqual("Aptos", document.styles["Normal"].font.name)
            self.assertEqual("Heading 1", next(p for p in document.paragraphs if p.text == "执行摘要").style.name)
            self.assertAlmostEqual(0.8, document.sections[0].left_margin.inches, places=2)
            with zipfile.ZipFile(output) as archive:
                document_xml = archive.read("word/document.xml").decode("utf-8")
                footer_xml = archive.read("word/footer1.xml").decode("utf-8")
                rels = archive.read("word/_rels/document.xml.rels").decode("utf-8")
            self.assertIn("tblHeader", document_xml)
            self.assertIn("cantSplit", document_xml)
            self.assertIn("蓝色收入趋势图", document_xml)
            self.assertIn(" PAGE ", footer_xml)
            self.assertIn("https://example.com/source", rels)

    def test_rejects_unknown_fields_and_missing_alt_text(self) -> None:
        with self.assertRaisesRegex(docx_renderer.DocxSpecError, "unknown field"):
            docx_renderer.validate_spec({"schemaVersion": 2, "blocks": [], "typo": True})
        with self.assertRaisesRegex(docx_renderer.DocxSpecError, "altText is required"):
            docx_renderer.validate_spec({
                "schemaVersion": 2,
                "blocks": [{"type": "image", "path": "image.png"}],
            })


if __name__ == "__main__":
    unittest.main()
