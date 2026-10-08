/**
 * The Chat area's welcome: the duck floating on a little pond of light, thought bubbles
 * drifting off its head, a greeting for the time of day and a few ideas to start from.
 * Typing makes the duck perk up and its bubbles scatter; sending makes it dive.
 * Motion lives in styles.css (`.chat-hero-*`), where reduced motion leaves a still scene.
 */
import { useContext, useMemo } from "react";
import { motion, useReducedMotion } from "motion/react";
import { AssistantNameContext } from "../../agentName";
import { DuckMark } from "../DuckMark";
import { greeting, pickIdeas, type PromptIdea } from "./chat-hero";
import { SPRING } from "./ChatBubbles";

interface ChatHeroProps {
  typing: boolean;
  /** A send is on its way: the duck dives, the greeting lifts away. */
  leaving: boolean;
  /** Changes per visit so the ideas rotate. */
  seed: number;
  onIdea: (idea: PromptIdea) => void;
}

export function ChatHero({ typing, leaving, seed, onIdea }: ChatHeroProps) {
  const name = useContext(AssistantNameContext);
  const reduce = useReducedMotion() ?? false;
  const ideas = useMemo(() => pickIdeas(seed), [seed]);
  const hello = useMemo(() => greeting(new Date().getHours()), []);
  return (
    <div className={`chat-hero${typing ? " typing" : ""}${leaving ? " leaving" : ""}`}>
      <div className="chat-hero-stage" aria-hidden="true">
        <span className="chat-hero-pond" />
        <span className="chat-hero-ripple" />
        <span className="chat-hero-ripple late" />
        <span className="chat-hero-duck"><DuckMark /></span>
        <span className="chat-hero-bubble one" />
        <span className="chat-hero-bubble two" />
        <span className="chat-hero-bubble three" />
        <span className="chat-hero-bubble four" />
        <span className="chat-hero-spark a" />
        <span className="chat-hero-spark b" />
      </div>
      <motion.h1 className="chat-hero-title"
        initial={reduce ? false : { opacity: 0, y: 14 }} animate={leaving && !reduce ? { opacity: 0, y: -48 } : { opacity: 1, y: 0 }}
        transition={leaving ? { duration: 0.24, ease: [0.33, 1, 0.68, 1] } : { ...SPRING, delay: 0.05 }}>
        {hello}! <span className="chat-hero-name">What's on your mind?</span>
      </motion.h1>
      <motion.p className="chat-hero-sub" initial={reduce ? false : { opacity: 0, y: 10 }} animate={leaving ? { opacity: 0 } : { opacity: 1, y: 0 }}
        transition={{ ...SPRING, delay: 0.12 }}>
        {name} is all ears. Ask, ramble, or think out loud.
      </motion.p>
      <div className="chat-hero-ideas" role="group" aria-label="Ideas to start with">
        {ideas.map((idea, index) => (
          <motion.button key={idea.label} type="button" className="chat-idea"
            initial={reduce ? false : { opacity: 0, y: 12, scale: 0.9 }}
            animate={leaving && !reduce ? { opacity: 0, y: 10, scale: 0.9 } : { opacity: 1, y: 0, scale: 1 }}
            transition={{ ...SPRING, delay: leaving ? 0 : 0.18 + index * 0.06 }}
            onClick={() => onIdea(idea)}>
            {idea.label}
          </motion.button>
        ))}
      </div>
    </div>
  );
}
