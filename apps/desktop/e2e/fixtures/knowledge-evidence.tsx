import { StrictMode, useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { StreamingMarkdown } from '../../src/components/chat/StreamingMarkdown';
import { CitationChip } from '../../src/components/chat/EvidenceCard';
import { FilePreviewContext } from '../../src/features/preview/filePreviewContext';
import { I18nProvider } from '../../src/i18n';
import '../../src/index.css';
import { EvidenceReader } from '../../src/features/preview/EvidenceReader';
import { ResearchWorkspace } from '../../src/features/knowledge/ResearchWorkspace';
import { StructuredPreviewRenderer, createPreviewLabels } from '../../src/features/preview/StructuredPreview';
import { useTranslation } from '../../src/i18n';
import type { EvidenceRef } from '../../src/types/evidence';

const extracted = new URLSearchParams(location.search).has('extracted');
const historical: EvidenceRef = {sourceId:'fixture-source',documentId:'fixture-doc',revision:'revision-2026-09',documentHash:'raw-hash-500',blockId:'old',contentHash:'old-500',locator:extracted ? {kind:'extracted',section:'Pages 1, 2'} : {kind:'pdf',page:2,extractedLineStart:7,extractedLineEnd:9},extractionMethod:extracted ? 'docling_layout_ocr' : 'native',status:extracted ? 'current' : 'historical'};
const ref = (id: string): EvidenceRef => ({...historical,blockId:id,contentHash:`hash-${id}`});
function ReaderFixture() {
  const [opened,setOpened] = useState(false);
  const [position,setPosition] = useState<unknown>(null);
  const context = useMemo(() => ({
    openFilePreview: (_path: string, location: unknown) => setPosition(location ?? 'current-file-without-historical-anchor'),
    openWebLink: () => undefined,
    loadEvidenceContext: async (reference: EvidenceRef) => {
      const body = {reference,cards:[{...fixture,chunkId:reference.blockId,content:reference.blockId === 'old' ? 'Original limit: 500 yuan.' : `Evidence section ${reference.blockId}`,evidenceRef:reference}],truncated:false};
      if ((window as any).__failEvidence) throw new Error('Context temporarily unavailable');
      if(reference.blockId === 'slow') return await new Promise<any>(resolve => { (window as any).__resolveSlow = () => resolve(body); });
      return body;
    },
    loadDocumentOutline: async (_reference: EvidenceRef, after?: number|null) => ({sections:after == null ? [{reference:historical,chunkIndex:0,heading:'Original version'},{reference:ref('slow'),chunkIndex:1,heading:'Slow section'},{reference:ref('fast'),chunkIndex:2,heading:'Fast section'}] : [{reference:ref('appendix'),chunkIndex:3,heading:'Appendix'}],hasMore:after == null,nextIndex:after == null ? 2 : null}),
  }),[]);
  return <FilePreviewContext.Provider value={context}><button onClick={() => setOpened(true)}>Open historical evidence</button><output data-testid="opened-position">{JSON.stringify(position)}</output>{opened && <EvidenceReader reference={historical} onClose={() => setOpened(false)} />}</FilePreviewContext.Provider>;
}
function FileLinkFixture() {
  const [opened, setOpened] = useState('');
  const hash = 'b'.repeat(64);
  return <FilePreviewContext.Provider value={{
    openFilePreview: (path, position) => setOpened(JSON.stringify({ path, position })),
    openWebLink: () => undefined,
  }}>
    <StreamingMarkdown content={`[Budget · L2–4](<file:///C:/notes/cost%20%23%25.md#L2-L4@${hash}>) and [file:/tmp/old.md:2-4|old.md]`} isStreaming={false} reduceMotion />
    <output data-testid="file-link-target">{opened}</output>
  </FilePreviewContext.Provider>;
}
function PdfLinesFixture() {
  const { t } = useTranslation();
  return <StructuredPreviewRenderer preview={{ type: 'document', assets: [], blocks: [
    { type: 'paragraph', extractedLineStart: 6, extractedLineEnd: 8, runs: [{ text: 'before\nselected\nafter', bold: false, italic: false, underline: false }] },
  ] }} locator={{ kind:'pdf', page:1, extractedLineStart:7, extractedLineEnd:7 }}
    labels={createPreviewLabels(t)} onMouseUp={() => undefined} onOpenWebLink={() => undefined} />;
}
function WorkbookFixture() {
  const {t}=useTranslation();
  return <StructuredPreviewRenderer preview={{type:'workbook',truncated:false,limits:{maxSheets:20,maxRows:500,maxColumns:60},sheets:[{name:'预算',index:0,startRow:2,startColumn:1,rowCount:2,columnCount:2,previewRowCount:2,previewColumnCount:2,truncated:false,mergedRanges:[],cells:[{row:0,column:0,value:'金额(元)',dataType:'text',formula:null},{row:0,column:1,value:'500',dataType:'number',formula:null},{row:1,column:0,value:'Total',dataType:'text',formula:null},{row:1,column:1,value:'17',dataType:'number',formula:'SUM(C3)'}]}]}} locator={{kind:'sheet',sheet:'预算',range:'C4',contextRange:'B3:C3'}} labels={createPreviewLabels(t)} onMouseUp={() => undefined} onOpenWebLink={() => undefined} />;
}
function ResearchFixture() {
  const [mounted,setMounted]=useState(true);
  const [opened,setOpened]=useState('');
  return <FilePreviewContext.Provider value={{openFilePreview:()=>undefined,openWebLink:()=>undefined,openEvidence:reference=>setOpened(reference.revision)}}><button onClick={()=>setMounted(value=>!value)}>Toggle workspace</button><output data-testid="research-opened">{opened}</output>{mounted && <ResearchWorkspace cards={[{...fixture,evidenceRef:{...historical,status:'current'}}]} sourceIds={[]} />}</FilePreviewContext.Provider>;
}
function DocxFixture() {
  const {t}=useTranslation();
  const run=(text:string)=>({text,bold:false,italic:false,underline:false});
  const table=new URLSearchParams(location.search).has('table');
  return <StructuredPreviewRenderer preview={{type:'document',assets:[],blocks:table
    ? [{type:'table',sourceTable:2,rows:[{sourceRow:1,cells:[{blocks:[{type:'paragraph',sourceParagraph:2,runs:[run('Header')]}]}]},{sourceRow:2,cells:[{blocks:[{type:'paragraph',sourceParagraph:3,runs:[run('Value')]}]}]}]}]
    : [{type:'paragraph',sourceParagraph:2,runs:[run('Approved')]},{type:'paragraph',sourceParagraph:3,runs:[run('Approved')]}]
  }} locator={table ? {kind:'document',part:'word/document.xml',paragraph:3,table:2,row:2,contextRow:1} : {kind:'document',part:'word/document.xml',paragraph:2}} focusText={table?'Value':'Approved'} labels={createPreviewLabels(t)} onMouseUp={()=>undefined} onOpenWebLink={()=>undefined} />;
}
const mode = new URLSearchParams(location.search).get('mode');


let attempts = 0;
const fixture = {
  chunkId: 'fixture-block', documentId: 'fixture-doc', sourceId: 'fixture-source',
  documentPath: 'C:/notes/report.md', documentTitle: 'Evidence restored', sourceName: 'Reports',
  content: 'A traceable statement from the source.', score: 0.8, headingPath: ['Results'], highlights: [],
};
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <I18nProvider>
    {mode === 'file-links' ? <FileLinkFixture /> : mode === 'pdf-lines' ? <PdfLinesFixture /> : mode === 'docx' ? <DocxFixture /> : mode === 'reader' ? <ReaderFixture /> : mode === 'workbook' ? <WorkbookFixture /> : mode === 'research' ? <ResearchFixture /> : <FilePreviewContext.Provider value={{
      openFilePreview: () => undefined,
      openWebLink: () => undefined,
      loadEvidence: async () => {
        attempts += 1;
        if (attempts === 1) throw new Error('Temporary evidence read failure');
        return { ...fixture, evidenceRef: { ...historical, blockId: fixture.chunkId } };
      },
    }}>
      <CitationChip chunkId="fixture-block" displayText="Read citation" card={undefined} />
    </FilePreviewContext.Provider>}
    </I18nProvider>
  </StrictMode>,
);
