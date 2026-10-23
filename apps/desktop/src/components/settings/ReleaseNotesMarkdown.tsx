import ReactMarkdown from 'react-markdown';
import {
  markdownComponents,
  markdownRemarkPlugins,
  rehypePlugins,
} from '../chat/markdownComponents';

/** Load the rich renderer only when the user opens the release notes. */
export default function ReleaseNotesMarkdown({ content }: { content: string }) {
  return (
    <ReactMarkdown
      remarkPlugins={markdownRemarkPlugins}
      rehypePlugins={rehypePlugins}
      components={markdownComponents}
    >
      {content}
    </ReactMarkdown>
  );
}
