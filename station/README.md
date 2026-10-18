# Station layout

```
MyStation\
  1 RUN HERE.bat        run the default job over INBOX (pass a job name to pick another)
  2 RUN ON FOLDER.bat   drag a folder or file on it: dry run first, then confirm
  3 START GATEWAY.bat   start the local HTTP gateway for the HTML pages / other tools
  BACKSIDE\
    forge.exe
    config\providers.json  config\settings.json     (from `forge init BACKSIDE`)
    templates\*.md                                    your job templates
    BRIEF.md                                          what another AI should know first (optional)
    _data\forge.sqlite                                ledger, document state, context library (created)
    LOGS\
  INBOX\   (INBOX\00_PRIORITY\ runs first)
  OUTBOX\  (OUTBOX\_older\<date>\ holds anything replaced; nothing is deleted)
```

Keys are environment variables only (`DEEPSEEK_API_KEY`, `ANTHROPIC_API_KEY`, `OPENROUTER_API_KEY`, `GEMINI_API_KEY`, ...).
Set them once with `setx DEEPSEEK_API_KEY "sk-..."` and open a new terminal.
