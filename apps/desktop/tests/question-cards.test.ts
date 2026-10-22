import {
  extractQuestionCards,
  extractQuestionRequest,
  formatQuestionResponse,
  questionAnswerComplete,
} from '../src/lib/questionCards';

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function assertEqual<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

const legacy = extractQuestionCards(`\`\`\`question-cards\r\n[{"id":"scope","question":"Which scope?","type":"single_choice","options":["App","Repo"]}]\r\n\`\`\``);
assertEqual(legacy.length, 1, 'legacy CRLF blocks render');
assertEqual(legacy[0].options?.[0]?.label, 'App', 'string options normalize to labels');
const sixQuestions = extractQuestionRequest('six-questions', {
  questions: Array.from({ length: 6 }, (_, index) => ({ id: `q${index}`, question: `Question ${index + 1}` })),
});
assertEqual(sixQuestions?.questions.length, 6, 'all six backend-supported questions survive display normalization');

const request = extractQuestionRequest(
  'call-question-1',
  JSON.stringify({
    questions: [{
      id: 'scope',
      header: 'Scope',
      question: 'Which scope?',
      type: 'single_choice',
      options: [{ label: 'App', description: 'Only this app.' }, { label: 'Repo', description: 'Whole repository.' }],
    }],
  }),
  {
    kind: 'questionRequest',
    version: 2,
    callId: 'call-question-1',
    interactionId: 'interaction-1',
    status: 'pending',
  },
);

assert(request, 'tool arguments produce a question request');
assertEqual(request.questions[0].options?.[1]?.description, 'Whole repository.', 'rich option descriptions survive');

const response = formatQuestionResponse(request, { scope: ['Repo'] });
assert(response.message.includes('Which scope?'), 'response includes question text');
assert(response.message.includes('Repo'), 'response includes selected answer');
assertEqual(response.artifact.kind, 'questionResponse', 'response carries a typed artifact');
assertEqual(response.artifact.requestCallId, 'call-question-1', 'response links to the tool call');
assertEqual(response.artifact.interactionId, 'interaction-1', 'response links to the durable interaction');
assertEqual(response.artifact.resumeToken, undefined, 'response does not persist the resume token');
assert(!questionAnswerComplete('scope', {}, true), 'untouched questions are unanswered');
assert(!questionAnswerComplete('scope', { scope: [' '] }, true), 'whitespace is not an explicit skip');
assert(questionAnswerComplete('scope', { scope: [] }, true), 'an explicit skip completes a normal question');
assert(!questionAnswerComplete('scope', { scope: [] }, false), 'confirmations cannot be skipped');
const skippedResponse = formatQuestionResponse(request, { scope: [] });
assert(skippedResponse.message.includes('Skipped by user'), 'the agent receives an explicit skip instead of fabricated input');
