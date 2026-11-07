import { createRoot } from 'react-dom/client';
import { I18nProvider } from '../../src/i18n';
import { StreamingMarkdown } from '../../src/components/chat/StreamingMarkdown';
import { FilePreviewContext } from '../../src/features/preview/filePreviewContext';
import '../../../mobile/src/mobile.css';

const ui = { version:1, title:'手机上的数据图表', blocks:[
  {type:'bars', title:'各团队任务变化', unit:'项任务', items:[{label:'移动端体验与无障碍适配团队',value:1250},{label:'桌面端与供应商支持',value:-125}]},
  {type:'tabs',tabs:[{label:'数据与方法说明',blocks:[{type:'table',columns:['团队','状态','下一步'],rows:[['移动端体验与无障碍适配团队','进行中','检查不同屏幕尺寸下的触摸操作和内容完整性']]}]},{label:'比较与后续计划',blocks:[{type:'text',text:'保留所有数据。'}]}]},
  {type:'metrics',items:[{label:'本期总计',value:'1,234,567,890',detail:'有较长数值的指标卡片'}]},
]};
const source = [
  '# 手机渲染验收',
  '中文、English 和长链接 https://example.test/' + 'verylongpath/'.repeat(10),
  '| 日期 | 项目 | 状态 | 下一步 | 负责人 |\n|---|---|---|---|---|\n|2026-11-05|手机适配与图表|进行中|检查宽度和操作|移动端体验团队|',
  '| 地区 | 叫法 |\n|---|---|\n|桐庐|江刺鱼|\n|川渝|黄辣丁|',
  '```ts\nconst endpoint = "https://example.test/' + 'longpath/'.repeat(20) + '";\n```',
  '```nexa-ui\n' + JSON.stringify(ui) + '\n```',
  '```mermaid\nflowchart LR\n A[输入] --> B[解析 Markdown 内容] --> C[渲染手机图表] --> D[验证触摸与尺寸]\n```',
  '```nexa-html\n<div style="width:960px;display:grid;grid-template-columns:1fr 1fr;gap:24px"><h1>HTML 图表</h1><svg width="900" height="240" viewBox="0 0 900 240"><rect width="900" height="100" fill="teal"/><text x="20" y="160" font-size="28">所有内容保持可见</text></svg></div>\n```',
].join('\n\n');
export function renderMobileRendering() {
  document.documentElement.dataset.remote = 'true'; localStorage.setItem('nexa-locale','zh-CN');
  const host=document.createElement('div');host.className='mobile-app';document.body.appendChild(host);
  createRoot(host).render(<I18nProvider><FilePreviewContext.Provider value={{openFilePreview(){},openWebLink(){},remote:true}}><main className="native-chat"><div className="native-chat-content"><article className="remote-answer"><StreamingMarkdown content={source} isStreaming={false} reduceMotion/></article></div></main></FilePreviewContext.Provider></I18nProvider>);
}
