module github.com/example/widget

go 1.21

require (
	github.com/google/uuid v1.6.0
	golang.org/x/text v0.14.0
)

require github.com/davecgh/go-spew v1.1.1 // indirect

replace golang.org/x/text => golang.org/x/text v0.13.0

exclude github.com/google/uuid v1.5.0
