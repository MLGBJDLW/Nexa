import { useCallback, useImperativeHandle, useLayoutEffect, useRef, type Ref, type HTMLAttributes } from 'react';
import { EditorState, Compartment } from '@codemirror/state';
import { EditorView, highlightActiveLine, keymap } from '@codemirror/view';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { defaultKeymap, history, historyKeymap, insertNewline } from '@codemirror/commands';
import { styleTags, Tag, tags } from '@lezer/highlight';

const delimiter = Tag.define();
const linkDestination = Tag.define();
const draftMarkdown = markdown({
  base: markdownLanguage, addKeymap: false, completeHTMLTags: false,
  extensions: [{ props: [styleTags({
    'HeaderMark EmphasisMark StrikethroughMark CodeMark LinkMark': delimiter,
    'Link/URL Image/URL Link/LinkTitle Image/LinkTitle': linkDestination,
  })] }],
});
const markdownStyle = HighlightStyle.define([
  { tag: tags.heading, class: 'draft-heading' },
  { tag: tags.strong, class: 'draft-strong' },
  { tag: tags.emphasis, class: 'draft-emphasis' },
  { tag: tags.strikethrough, class: 'draft-strike' },
  { tag: tags.monospace, class: 'draft-code' },
  { tag: [tags.link, tags.url], class: 'draft-link' },
  { tag: tags.quote, class: 'draft-quote' },
  { tag: delimiter, class: 'draft-delimiter' },
  { tag: linkDestination, class: 'draft-destination' },
]);
const format = [draftMarkdown, syntaxHighlighting(markdownStyle), EditorView.lineWrapping,
  highlightActiveLine(), history(), keymap.of([{ key: 'Shift-Enter', run: insertNewline }, ...historyKeymap, ...defaultKeymap])];

export interface DraftEditorHandle {
  focus(): void;
  getSelection(): { from: number; to: number };
  setCursor(position: number): void;
}

interface Props extends Omit<HTMLAttributes<HTMLDivElement>, 'onChange' | 'onSelect'> {
  editorRef: Ref<DraftEditorHandle>;
  draftKey: string;
  value: string;
  label: string;
  description?: string;
  placeholder: string;
  testId?: string;
  disabled?: boolean;
  onChange(value: string): void;
  onCursor?(position: number): void;
}

export function MarkdownDraftEditor({ editorRef, draftKey, value, label, description, placeholder, testId,
  disabled = false, onChange, onCursor, className = '', ...events }: Props) {
  const mount = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const current = useRef({ value, onChange, onCursor });
  current.current = { value, onChange, onCursor };
  const attributes = useRef(new Compartment()).current;
  const lastValue = useRef(value);

  useImperativeHandle(editorRef, () => ({
    focus() { viewRef.current?.focus(); },
    getSelection() { return viewRef.current?.state.selection.main ?? { from: 0, to: 0 }; },
    setCursor(position) {
      const view = viewRef.current;
      if (view) view.dispatch({ selection: { anchor: Math.min(position, view.state.doc.length) }, scrollIntoView: true });
    },
  }), []);

  const createState = useCallback((doc: string, anchor = 0) => EditorState.create({
    doc, selection: { anchor },
    extensions: [format, attributes.of(viewRef.current ? attributes.get(viewRef.current.state) ?? [] : []),
      EditorView.updateListener.of(update => {
        if (update.docChanged) {
          const text = update.state.doc.toString();
          lastValue.current = text;
          current.current.onChange(text);
        }
        if (update.docChanged || update.selectionSet) current.current.onCursor?.(update.state.selection.main.from);
      })],
  }), [attributes]);

  useLayoutEffect(() => {
    const view = new EditorView({ parent: mount.current!, state: createState(current.current.value) });
    viewRef.current = view;
    lastValue.current = current.current.value;
    return () => { viewRef.current = null; view.destroy(); };
  }, [createState, draftKey]);

  useLayoutEffect(() => {
    const view = viewRef.current!;
    view.dispatch({ effects: attributes.reconfigure([
      EditorView.editable.of(!disabled), EditorState.readOnly.of(disabled),
      EditorView.contentAttributes.of({
        'aria-label': label, 'aria-multiline': 'true', 'aria-disabled': String(disabled),
        'aria-placeholder': placeholder, 'data-draft-key': draftKey,
        ...(testId && { 'data-testid': testId }), ...(description && { 'aria-description': description }),
      }),
    ]) });
  }, [attributes, draftKey, label, description, placeholder, testId, disabled]);

  useLayoutEffect(() => {
    const view = viewRef.current!;
    if (value === lastValue.current) return;
    // External replacements (history, dictation, send and draft restoration) start a fresh undo history.
    lastValue.current = value;
    const anchor = Math.min(view.state.selection.main.head, value.length);
    view.setState(createState(value, anchor));
  }, [createState, value]);

  return <div {...events} ref={mount} className={`markdown-draft-editor ${className}`} data-empty={value.length === 0} />;
}
