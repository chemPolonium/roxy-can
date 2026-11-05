param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string[]]$Path,
    [string]$Schema = "target/refs/AUTOSAR_00052.xsd",
    [string]$XmlSchema = "target/refs/xml.xsd",
    [int]$MaxErrors = 20
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Xml

if (-not (Test-Path -LiteralPath $Schema)) {
    throw "schema not found: $Schema"
}
if (-not (Test-Path -LiteralPath $XmlSchema)) {
    $fallback = "target/refs/ParseX/ParseX-main/resources/schemas/xml.xsd"
    if (Test-Path -LiteralPath $fallback) {
        $XmlSchema = $fallback
    } else {
        throw "xml.xsd not found: $XmlSchema"
    }
}

$schemas = [System.Xml.Schema.XmlSchemaSet]::new()
$schemas.Add("http://www.w3.org/XML/1998/namespace", $XmlSchema) | Out-Null
$schemas.Add("http://autosar.org/schema/r4.0", $Schema) | Out-Null
$schemas.Compile()

$failed = 0
foreach ($file in $Path) {
    $errors = [System.Collections.Generic.List[string]]::new()
    $settings = [System.Xml.XmlReaderSettings]::new()
    $settings.ValidationType = [System.Xml.ValidationType]::Schema
    $settings.Schemas = $schemas
    $settings.ValidationFlags = [System.Xml.Schema.XmlSchemaValidationFlags]::ReportValidationWarnings
    $handler = [System.Xml.Schema.ValidationEventHandler] {
        param($sender, $event)
        $errors.Add($event.Message)
    }
    $settings.add_ValidationEventHandler($handler)
    try {
        $reader = [System.Xml.XmlReader]::Create($file, $settings)
        while ($reader.Read()) { }
        $reader.Close()
    } catch {
        $errors.Add($_.Exception.Message)
    }
    if ($errors.Count -eq 0) {
        Write-Output "OK   $file"
    } else {
        $failed++
        Write-Output "FAIL $file ($($errors.Count) errors)"
        $errors | Select-Object -First $MaxErrors | ForEach-Object {
            Write-Output "  $_"
        }
    }
}
exit $(if ($failed -eq 0) { 0 } else { 1 })
