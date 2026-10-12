import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nProvider } from '../../src/i18n';
import { StreamingMarkdown } from '../../src/components/chat/StreamingMarkdown';
import { FilePreviewContext } from '../../src/features/preview/filePreviewContext';
import sample from '../../../../crates/core/assets/skills/visual-explanations/resources/examples.json';

const initial = '```nexa-ui\n'+JSON.stringify(sample)+'\n```';
function Fixture() {
  const [source,setSource]=useState(localStorage.getItem('ui-fixture-source') || initial);
  const [streaming,setStreaming]=useState(false); const [visible,setVisible]=useState(true);
  Object.assign(window,{setUiFixture:(text:string,live=false)=>{localStorage.setItem('ui-fixture-source',text);setSource(text);setStreaming(live);},hideUiFixture:()=>setVisible(false)});
  return <main className="mx-auto max-w-3xl p-4 text-text-primary"><h1 className="text-lg font-medium">Nexa · Chat</h1><p className="my-2 text-xs text-text-secondary">Generative conversation acceptance</p>{visible && <StreamingMarkdown content={source} isStreaming={streaming} reduceMotion />}</main>;
}
const context = {
  openFilePreview() {}, openWebLink() {},
  async createHtmlPreview(html:string) {
    const next=await (window as any).publishUiFixture(html);
    return {url:next.url, release:()=>{void (window as any).releaseUiFixture(next.id);}};
  },
};
export function renderFixture(){ createRoot(document.body.appendChild(document.createElement('div'))).render(<I18nProvider><FilePreviewContext.Provider value={context}><Fixture /></FilePreviewContext.Provider></I18nProvider>); }
