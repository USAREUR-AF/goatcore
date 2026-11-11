module github.com/usareur-af/goatcore/ops/wg-cp0-mesh-reach

go 1.26.11

require (
	filippo.io/age v1.3.1
	github.com/usareur-af/goatcore/ops/enrollment v0.0.0-00000000000000-000000000000
	golang.org/x/crypto v0.58.0
)

replace github.com/usareur-af/goatcore/ops/enrollment => ../enrollment

require (
	filippo.io/hpke v0.4.0 // indirect
	github.com/fxamacker/cbor/v2 v2.9.1 // indirect
	github.com/go-piv/piv-go/v2 v2.6.0 // indirect
	github.com/x448/float16 v0.8.4 // indirect
	golang.org/x/sys v0.49.0 // indirect
)
