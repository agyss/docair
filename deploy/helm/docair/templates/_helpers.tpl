{{/* Labels on every object of the chart. */}}
{{- define "docair.labels" -}}
app.kubernetes.io/part-of: docair
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ .Chart.Name }}-{{ .Chart.Version }}
{{- end }}

{{/* Selector labels of one app; the argument is the app name. */}}
{{- define "docair.selector" -}}
app.kubernetes.io/name: {{ . }}
app.kubernetes.io/part-of: docair
{{- end }}
