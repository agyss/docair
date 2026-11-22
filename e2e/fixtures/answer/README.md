# answer

Someone asks a question. The app sums up its notes, asks the model with the
question and the summary, and brings the model's answer back.

```docair
Someone who asks a question.

@id      asker
@kind    external
@part-of answer
@calls   app.ask "asks" (question) -> answer
```

```docair
The app that answers.

@id      app
@kind    module
@part-of answer
```

```docair
Answers a question: sums up the notes, then asks the model.

@id      app.ask
@kind    function
@part-of app
@calls   app.summarize "sums up the notes" (notes) -> String
@calls   model "asks the model" (question + summary) -> answer
```

```docair
Sums up the notes in a few lines.

@id      app.summarize
@kind    function
@part-of app
@creates data.summary "sums up the notes"
```

```docair
Mails the weekly report.

@id      app.report
@kind    function
@part-of app
@calls   mail "mails the report" (summary)
```

```docair
The model that writes the answer.

@id      model
@kind    external
@part-of answer
@creates data.answer "writes the answer"
```

```docair
The mail server.

@id      mail
@kind    external
@part-of answer
```

```docair
The notes, summed up in a few lines.

@id      data.summary
@kind    data
@part-of answer
```

```docair
What the model answers.

@id      data.answer
@kind    data
@part-of answer
```
