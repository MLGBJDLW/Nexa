import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nProvider } from '../../src/i18n';
import { OverlayProvider } from '../../src/components/ui/overlay';
import { EmbeddingConfigSection } from '../../src/components/settings/EmbeddingConfigSection';
import type { EmbedderConfig } from '../../src/types/embedder';
import type { EmbeddingEstimate } from '../../src/types/ingest';
import '../../src/index.css';

function Fixture() {
  const [config, setConfig] = useState<EmbedderConfig>({ provider: 'api', apiKey: '', apiBaseUrl: 'https://api.openai.com/v1', apiModel: 'text-embedding-3-small', vectorDimensions: 1536, localModel: 'MultilingualMiniLM', modelPath: '' });
  const [tested, setTested] = useState<EmbedderConfig>();
  const [rebuilding, setRebuilding] = useState(false);
  const basis = new URLSearchParams(location.search).get('timing') as EmbeddingEstimate['basis'] | null;
  return <div className="mx-auto max-w-3xl p-6 text-text-primary">
    <EmbeddingConfigSection embedConfig={config} onConfigChange={setConfig} localModelReady={true}
      testLoading={false} embedSaveLoading={false} rebuildEmbedLoading={rebuilding} embedRebuildProgress={basis ? {
        sourceId: 'all', phase: 'embedding', current: 8, total: 80, currentFile: null,
        embedding: { model: 'qwen3.7-text-embedding', elapsedSeconds: 15, estimatedRemainingSeconds: basis === 'calibrating' ? null : 125, chunksPerSecond: 0.8, basis },
      } : null}
      agentConfigs={[]} onMarkDirty={() => {}} onTestConnection={setTested} onSave={setTested} onRebuild={() => {
        setRebuilding(true);
        Object.assign(window, { embeddingReady: true });
        setTimeout(() => setRebuilding(false), 200);
      }} />
    <output data-testid="embedding-config">{JSON.stringify(config)}</output>
    <output data-testid="embedding-tested">{JSON.stringify(tested)}</output>
  </div>;
}

createRoot(document.getElementById('root')!).render(<I18nProvider><OverlayProvider><Fixture /></OverlayProvider></I18nProvider>);
