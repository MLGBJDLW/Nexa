"""Optional loopback PDF layout/OCR service for Nexa (protocol 1).

Install docling in a separate Python environment, then run this file.
Nexa does not start this service or download its models automatically.
"""

import argparse
import hashlib
import importlib.metadata
import json
import os
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path


def resolved_source_path(value: Path) -> Path:
    """Compare Rust's Windows verbatim paths with ordinary configured roots."""
    text = str(value.resolve(strict=True))
    if text.startswith("\\\\?\\UNC\\"):
        text = "\\\\" + text[8:]
    elif text.startswith("\\\\?\\"):
        text = text[4:]
    return Path(text)


def convert_pdf(path: Path, cache: Path, converter, languages: list[str]) -> dict:
    from docling_core.types.doc import TableItem, TextItem

    result = converter.convert(path)
    if str(result.status.value) != "success":
        raise ValueError(f"Docling did not finish the document: {result.status.value}")
    doc = result.document
    blocks = []
    heading = None
    for item, _level in doc.iterate_items():
        if isinstance(item, TableItem):
            text = item.export_to_markdown(doc=doc)
        elif isinstance(item, TextItem):
            text = item.text
            if item.label.value in ("section_header", "title"):
                heading = text
        else:
            continue
        if not text.strip():
            continue
        if not item.prov:
            raise ValueError("PDF block has no page provenance")
        # A table may span pages. Repeat its full context with each true location.
        for prov in item.prov:
            page = doc.pages[prov.page_no]
            box = prov.bbox.to_top_left_origin(page_height=page.size.height)
            blocks.append({
                "text": text,
                "page": prov.page_no,
                "bbox": [box.l / page.size.width, box.t / page.size.height,
                         box.r / page.size.width, box.b / page.size.height],
                "heading": heading,
            })
    cache.mkdir(parents=True, exist_ok=True)
    digest = hashlib.sha256(path.read_bytes() + ",".join(languages).encode()).hexdigest()
    # Keep the complete structure for inspection; the index owns normalized blocks.
    doc.save_as_json(cache / f"{digest}.docling.json")
    return {"protocol": 1, "parserVersion": importlib.metadata.version("docling") + ":" + ",".join(languages),
            "pageCount": len(doc.pages), "blocks": blocks}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8091)
    parser.add_argument("--cache", type=Path, default=Path.home() / ".cache" / "nexa-docling")
    parser.add_argument("--root", type=Path, action="append", required=True,
                        help="Allowed PDF directory; repeat for multiple sources")
    parser.add_argument("--ocr-languages", default="en,ch_sim",
                        help="EasyOCR codes; default is English and simplified Chinese")
    parser.add_argument("--allow-model-downloads", action="store_true",
                        help="Explicitly allow first-use downloads in this separate environment")
    args = parser.parse_args()
    roots = [resolved_source_path(root) for root in args.root]
    languages = [value.strip() for value in args.ocr_languages.split(",") if value.strip()]
    if not args.allow_model_downloads:
        os.environ["HF_HUB_OFFLINE"] = "1"
    from docling.datamodel.base_models import InputFormat
    from docling.datamodel.pipeline_options import EasyOcrOptions, PdfPipelineOptions
    from docling.document_converter import DocumentConverter, PdfFormatOption
    options = PdfPipelineOptions(ocr_options=EasyOcrOptions(
        lang=languages, download_enabled=args.allow_model_downloads))
    converter = DocumentConverter(format_options={
        InputFormat.PDF: PdfFormatOption(pipeline_options=options)})

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self) -> None:
            if self.path != "/parse" or self.headers.get("Origin"):
                self.send_error(403)
                return
            try:
                length = int(self.headers.get("Content-Length", "0"))
                if not 0 < length <= 32_768:
                    raise ValueError("Invalid request size")
                request = json.loads(self.rfile.read(length))
                path = resolved_source_path(Path(request["path"]))
                if path.suffix.lower() != ".pdf" or not any(path.is_relative_to(root) for root in roots):
                    raise ValueError("PDF is outside configured source roots")
                if path.stat().st_size > 100 * 1024 * 1024:
                    raise ValueError("PDF exceeds 100 MiB")
                payload = json.dumps(convert_pdf(path, args.cache, converter, languages), ensure_ascii=False).encode("utf-8")
                if len(payload) > 16 * 1024 * 1024:
                    raise ValueError("Parsed document exceeds 16 MiB")
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)
            except Exception as error:
                # Paths/document text are never sent to arbitrary browser origins.
                self.send_error(422, str(error).replace("\n", " ")[:180])

    # Keep model work serial and offline once its artifacts have been provisioned.
    print(f"Nexa Docling service: http://127.0.0.1:{args.port}/parse", flush=True)
    HTTPServer(("127.0.0.1", args.port), Handler).serve_forever()


if __name__ == "__main__":
    main()
