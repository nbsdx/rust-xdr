initSidebarItems({"mod":[["pp","This pretty-printer is a direct reimplementation of Philip Karlton's Mesa pretty-printer, as described in appendix A ofThe algorithm's aim is to break a stream into as few lines as possible while respecting the indentation-consistency requirements of the enclosing block, and avoiding breaking at silly places on block boundaries, for example, between \"x\" and \")\" in \"x)\".I am implementing this algorithm because it comes with 20 pages of documentation explaining its theory, and because it addresses the set of concerns I've seen other pretty-printers fall down on. Weirdly. Even though it's 32 years old. What can I say?Despite some redundancies and quirks in the way it's implemented in that paper, I've opted to keep the implementation here as similar as I can, changing only what was blatantly wrong, a typo, or sufficiently non-idiomatic rust that it really stuck out.In particular you'll see a certain amount of churn related to INTEGER vs. CARDINAL in the Mesa implementation. Mesa apparently interconverts the two somewhat readily? In any case, I've used usize for indices-in-buffers and ints for character-sizes-and-indentation-offsets. This respects the need for ints to \"go negative\" while carrying a pending-calculation balance, and helps differentiate all the numbers flying around internally (slightly).I also inverted the indentation arithmetic used in the print stack, since the Mesa implementation (somewhat randomly) stores the offset on the print stack in terms of margin-col rather than col itself. I store col.I also implemented a small change in the String token, in that I store an explicit length for the string. For most tokens this is just the length of the accompanying string. But it's necessary to permit it to differ, for encoding things that are supposed to \"go on their own line\" -- certain classes of comment and blank-line -- where relying on adjacent hardbreak-like Break tokens with long blankness indication doesn't actually work. To see why, consider when there is a \"thing that should be on its own line\" between two long blocks, say functions. If you put a hardbreak after each function (or before each) and the breaking algorithm decides to break there anyways (because the functions themselves are long) you wind up with extra blank lines. If you don't put hardbreaks you can wind up with the \"thing which should be on its own line\" not getting its own line in the rare case of \"really small functions\" or such. This re-occurs with comments and explicit blank lines. So in those cases we use a string with a payload we want isolated to a line and an explicit length that's huge, surrounded by two zero-length breaks. The algorithm will try its best to fit it on a line (which it can't) and so naturally place the content on its own line to avoid combining it with other lines and making matters even worse."],["pprust",""]]});