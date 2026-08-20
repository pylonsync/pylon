# Pylon docs writing standard

Write every doc page in ASD-STE100 Simplified Technical English, with the
no-ai-slop rules applied. This page defines both. It is the reference for
authors and for review.

## Simplified Technical English (ASD-STE100)

- Write short sentences. Keep an instruction to 20 words. Keep a description to
  25 words.
- Give one instruction in one sentence.
- Use the active voice. Write "the build failed", not "the build was found to
  have failed".
- Use the present tense when you can.
- Use articles. Write "the token", not "token".
- Use one word for one meaning. Do not cycle synonyms for style.
- Use a vertical list for three or more items.
- Keep a paragraph to six sentences or fewer.
- Do not use jargon, idiom, or slang.

Pick one word per meaning and keep it across the docs:

- move — relocate an app from one host to another.
- rewrite — change code from one form to another.
- convert — change data from one form to another.
- map — state that concept A corresponds to concept B.

## no-ai-slop

Cut these words: delve, foster, leverage, utilize, facilitate, empower,
streamline, robust, cutting-edge, seamless, tapestry, realm, beacon,
multifaceted, meticulous, intricate, paramount, transformative, elevate, embark,
supercharge, harness, ever-evolving.

Cut these phrases when they delay the point: it's worth noting, it's important to
note, at the end of the day, when it comes to, at its core, in today's world, in
the world of, the reality is, the truth is, in terms of, going forward, in this
guide, let's dive in.

Cut these patterns:

- Binary contrast ("This is not X. It's Y."). State Y.
- Colon reveal ("The detail that makes it work: a second agent."). Write a plain
  sentence.
- Faux insight ("What most people get wrong."). Make the claim alone.
- Importance puffery ("marks a pivotal moment."). State the fact.
- Trailing `-ing` analysis ("…, highlighting the team's commitment.").
- Fake-profound last line. End on the last concrete point.
- Summary endings ("In conclusion.", "Ultimately.", "Overall.").
- Weasel attribution ("experts agree", "studies show"). Name the source or cut
  the claim.

## Formatting

- Do not put emoji in a heading.
- Do not use bold in the middle of a sentence for emphasis.
- Do not put a small uppercase label above a heading. The heading works alone.
- Use a header only when the section has more than two sentences.
- Use em dashes rarely. Prefer commas, periods, or parentheses. Use none in short
  copy.

## What to keep unchanged

- Frontmatter keys (`title`, `description`). You may sharpen the description
  text, but keep it a valid one-line description.
- Code blocks. Do not change code, flags, endpoints, or values.
- MDX components, links, and images.
- The technical facts. Rewrite the prose, not the API.
