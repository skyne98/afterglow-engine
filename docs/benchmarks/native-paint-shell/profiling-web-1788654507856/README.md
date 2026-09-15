# Initial browser WebSocket capture

The collector retained 53 valid records: 40 stroke samples and 13 worker messages.
Source identity, sequence continuity, and all loss counters passed the CLI checks.
The paint probe counted 954 colored pixels.

The test failed afterward because `Page.captureScreenshot` reached its deadline.
That extra screenshot is not part of the transport check in the corrected driver.

Do not use this run for capture-overhead measurements.
The initial result writer replaced the capture timing field with file metadata.
The corrected driver keeps separate `capture` and `captureFile` fields.
The post-capture sample also contains intervals above one second.
This run does not identify their cause.
