Provider SVG masks generated from @lobehub/icons Mono components, including ElevenLabs and Jina.

Source: https://github.com/lobehub/lobe-icons
License: MIT

Added XiaomiMiMo, Cohere, Meta, Groq, Inception, Inference, Sakana and AntGroup
from the same Lobe Icons collection (2026-09-23). SVGs are rendered as
currentColor masks for both light and dark themes.

Additional brand assets, used only to identify the corresponding model publisher:
- PrismML: https://prismml.com/ (official prism-logo.svg, symbol viewBox crop)
- Unbiased: https://unbiased.ai/assets/logo-icon.svg
- Nex-AGI: https://nex-agi.com/logo/nex.svg

Those three brands retain their respective owners' trademark rights.

External-agent SVGs in `agents/` are bundled from the official ACP registry.
`agents/sources.json` records each source URL and SHA-256. They are rendered as
local monochrome masks and never fetched from a third party at runtime. Hermes
uses its official `assets/icon-master.svg` with the white background removed
for mask rendering; the custom-agent glyph is original Nexa artwork. These
assets identify the corresponding agents; their owners retain trademark rights.
Registry imports require the reviewed SVG to be present before importing an agent.

Flux, Krea, Microsoft and Recraft also use the Lobe Icons MIT Mono assets.

Sourceful: official header emblem from https://www.sourceful.com/ (50x50 symbol
crop). The brand retains its owner's trademark rights.

Vertex AI and Amazon Bedrock use the Lobe Icons MIT Mono SVGs (2026-10-19).
