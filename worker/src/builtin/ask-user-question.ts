/**
 * Built-in `ask_user_question` tool — a structured multi-question dialog the model can raise
 * mid-task, rendered natively by the desktop. Adapted from the question tool in
 * `@narumitw/pi-plan-mode` v0.58.3 (MIT), generalized to be available in every mode and to
 * render through WackCode's UI bridge instead of the terminal.
 *
 * In Ultra Plan the card also offers "Write the plan now". Once the user presses it, the rest
 * of that run gets the same instruction back without a dialog, so an eager model can't keep
 * interviewing; the next user message lifts it.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import type { AskQuestion, AskQuestionOption, QuestionAnswer } from "../protocol.js";
import type { BuiltinHost } from "./host.js";

export const ASK_USER_QUESTION_TOOL_NAME = "ask_user_question";
export const MAX_ANSWER_LENGTH = 4_000;

export const ASK_USER_QUESTION_PARAMS = {
  type: "object",
  additionalProperties: false,
  required: ["questions"],
  properties: {
    questions: {
      type: "array",
      minItems: 1,
      maxItems: 3,
      description: "Questions to ask the user. Prefer 1 and do not exceed 3.",
      items: {
        type: "object",
        additionalProperties: false,
        required: ["id", "header", "question", "options"],
        properties: {
          id: {
            type: "string",
            description: "Stable snake_case identifier for mapping answers.",
          },
          header: {
            type: "string",
            description: "Short tab label shown in the UI (12 or fewer characters).",
          },
          question: { type: "string", description: "Single-sentence question shown to the user." },
          multiSelect: {
            type: "boolean",
            description: "Allow selecting several options instead of exactly one. Default false.",
          },
          options: {
            type: "array",
            minItems: 2,
            maxItems: 4,
            description:
              "Provide 2-4 mutually exclusive choices. Put the recommended option first when there is a clear default. The user can always pick a free-form Other answer instead.",
            items: {
              type: "object",
              additionalProperties: false,
              required: ["label", "description"],
              properties: {
                label: { type: "string", description: "User-facing label (1-5 words)." },
                description: {
                  type: "string",
                  description: "One short sentence explaining impact/tradeoff if selected.",
                },
              },
            },
          },
        },
      },
    },
  },
} as const;

type NormalizeResult = { ok: true; questions: AskQuestion[] } | { ok: false; error: string };

export function normalizeAskQuestionsParams(input: unknown): NormalizeResult {
  if (!isRecord(input) || !Array.isArray(input.questions)) {
    return { ok: false, error: "questions must be an array" };
  }
  if (input.questions.length < 1 || input.questions.length > 3) {
    return { ok: false, error: "questions must contain 1-3 items" };
  }
  const questions: AskQuestion[] = [];
  for (const [questionIndex, rawQuestion] of input.questions.entries()) {
    if (!isRecord(rawQuestion)) {
      return { ok: false, error: `question ${questionIndex + 1} must be an object` };
    }
    const id = stringField(rawQuestion.id);
    const header = stringField(rawQuestion.header);
    const question = stringField(rawQuestion.question);
    if (!id || !header || !question) {
      return { ok: false, error: `question ${questionIndex + 1} requires non-empty id, header, and question` };
    }
    if (!Array.isArray(rawQuestion.options)) {
      return { ok: false, error: `question ${questionIndex + 1} options must be an array` };
    }
    if (rawQuestion.options.length < 2 || rawQuestion.options.length > 4) {
      return { ok: false, error: `question ${questionIndex + 1} options must contain 2-4 items` };
    }
    const options: AskQuestionOption[] = [];
    for (const [optionIndex, rawOption] of rawQuestion.options.entries()) {
      if (!isRecord(rawOption)) {
        return { ok: false, error: `question ${questionIndex + 1} option ${optionIndex + 1} must be an object` };
      }
      const label = stringField(rawOption.label);
      const description = stringField(rawOption.description);
      if (!label || !description) {
        return {
          ok: false,
          error: `question ${questionIndex + 1} option ${optionIndex + 1} requires a label and a description`,
        };
      }
      options.push({ label, description });
    }
    const entry: AskQuestion = { id, header, question, options };
    if (rawQuestion.multiSelect === true) entry.multiSelect = true;
    questions.push(entry);
  }
  return { ok: true, questions };
}

interface QuestionResultDetails {
  cancelled: boolean;
  reason?: "cancelled" | "ui_unavailable" | "invalid_input" | "wrap_up";
  questions: AskQuestion[];
  answers?: QuestionAnswer[];
}

function questionsAnswered(questions: AskQuestion[], answers: QuestionAnswer[]) {
  return {
    content: [{ type: "text" as const, text: formatAnswers(questions, answers) }],
    details: { cancelled: false, questions, answers } satisfies QuestionResultDetails,
  };
}

function questionsCancelled(reason: QuestionResultDetails["reason"], message: string) {
  return {
    content: [{ type: "text" as const, text: `${message} Ask concisely in plain text or proceed only with a clearly stated low-risk assumption.` }],
    details: { cancelled: true, reason, questions: [] } satisfies QuestionResultDetails,
  };
}

const WRAP_UP_INSTRUCTION =
  "The user wants to stop answering questions and review the plan now. Do not call ask_user_question again this turn. " +
  "Submit the complete plan with plan_mode_complete, resolving every open decision (including any you just asked about) " +
  "with your recommended answer and listing those as explicit assumptions.";

function questionsWrappedUp() {
  return {
    content: [{ type: "text" as const, text: WRAP_UP_INSTRUCTION }],
    details: { cancelled: true, reason: "wrap_up", questions: [] } satisfies QuestionResultDetails,
  };
}

function formatAnswers(questions: AskQuestion[], answers: QuestionAnswer[]): string {
  const lines = ["The user answered your questions:"];
  for (const question of questions) {
    const answer = answers.find((entry) => entry.questionId === question.id);
    if (!answer) continue;
    lines.push("", `**${question.header}** — ${question.question}`);
    for (const selected of answer.selected) lines.push(`- ${selected}`);
    if (answer.custom) lines.push(`- Other: ${answer.custom}`);
  }
  return lines.join("\n");
}

function stringField(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * The `ask_user_question` tool, registered as a built-in extension. `interviewing` says whether
 * Ultra Plan is on, which offers the wrap-up button.
 */
export function createAskUserQuestionExtension(host: BuiltinHost, interviewing: () => boolean = () => false) {
  return function askUserQuestion(pi: ExtensionAPI) {
    /** "Write the plan now" was pressed during the current run. */
    let wrapUpRequested = false;
    const resetWrapUp = () => {
      wrapUpRequested = false;
    };
    pi.on("before_agent_start", resetWrapUp);
    pi.on("session_start", resetWrapUp);
    pi.on("session_tree", resetWrapUp);

    pi.registerTool({
      name: ASK_USER_QUESTION_TOOL_NAME,
      label: "Ask user",
      description:
        "Ask the user 1-3 structured questions with meaningful preset options when a preference, tradeoff, or missing detail materially changes what you would do. Do not ask about facts you can discover from the repository or system.",
      promptSnippet: "ask the user structured multiple-choice questions when a decision is ambiguous",
      promptGuidelines: [
        "Use ask_user_question for important choices the codebase cannot answer: approach, scope, UX preferences. Each question needs 2-4 mutually exclusive options; the user can always answer with free-form text instead.",
        "Do not use ask_user_question for facts discoverable from the repository, trivial decisions, or confirmations of an obvious next step.",
      ],
      parameters: ASK_USER_QUESTION_PARAMS,
      async execute(_toolCallId, params: unknown, _signal, _onUpdate, ctx) {
        const parsed = normalizeAskQuestionsParams(params);
        if (!parsed.ok) {
          return questionsCancelled("invalid_input", `Error: ${parsed.error}.`);
        }
        const offerWrapUp = interviewing();
        if (offerWrapUp && wrapUpRequested) return questionsWrappedUp();
        if (!ctx.hasUI) {
          return questionsCancelled("ui_unavailable", "Interactive UI is not available, so the questions could not be shown.");
        }
        const answers = await host.askQuestions(parsed.questions, { offerWrapUp });
        if (answers === "wrap_up") {
          wrapUpRequested = true;
          return questionsWrappedUp();
        }
        if (!answers) {
          return questionsCancelled("cancelled", "The user dismissed the questions without answering.");
        }
        return questionsAnswered(parsed.questions, answers);
      },
    });
  };
}
