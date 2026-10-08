using System.Globalization;
using System.Text.Json;
using Apache.Arrow;
using Apache.Arrow.Ipc;
using Apache.Arrow.Types;
using Microsoft.AnalysisServices.AdomdClient;
using Microsoft.AnalysisServices.Tabular;
using JsonSerializer = System.Text.Json.JsonSerializer;

using var adapter = new EngineAdapter();
string? line;
while ((line = Console.ReadLine()) is not null)
{
    JsonElement id = JsonSerializer.SerializeToElement<object?>(null);
    try
    {
        using var request = JsonDocument.Parse(line);
        id = request.RootElement.GetProperty("id").Clone();
        var command = request.RootElement.GetProperty("command").GetString();
        var parameters = request.RootElement.GetProperty("params");
        var result = adapter.Execute(command, parameters);
        Console.WriteLine(JsonSerializer.Serialize(new { id, result }));
    }
    catch (Exception error)
    {
        // Connection strings and authentication tokens never go to stdout/stderr.
        Console.WriteLine(JsonSerializer.Serialize(new { id, error = new { code = "adapter_error", message = SafeMessage(error) } }));
    }
}

static string SafeMessage(Exception error)
{
    if (error is ArgumentException or InvalidOperationException or JsonException or NotSupportedException)
        return error.Message;
    return $"{error.GetType().Name}: engine request failed. Check the endpoint, model, query and authentication.";
}

sealed class EngineAdapter : IDisposable
{
    private AdomdConnection? connection;
    private AdomdConnection Connection
    {
        get
        {
            if (connection is not null) return connection;
            var value = Environment.GetEnvironmentVariable("LIGHTINGDAX_CONNECTION");
            if (string.IsNullOrWhiteSpace(value)) throw new InvalidOperationException("Set LIGHTINGDAX_CONNECTION to a Desktop or XMLA connection string.");
            var candidate = new AdomdConnection(value);
            try { candidate.Open(); connection = candidate; return candidate; }
            catch { candidate.Dispose(); throw; }
        }
    }
    public object Execute(string? command, JsonElement parameters) => command switch
    {
        "load_model" => LoadModel(Required(parameters, "path")),
        "run_query" => Query(Required(parameters, "query"), null, true),
        "snapshot_table" => Snapshot(Required(parameters, "table"), Required(parameters, "path")),
        "list_functions" => ListFunctions(),
        "engine_info" => new { version = Connection.ServerVersion },
        _ => throw new ArgumentException("Unknown adapter command.")
    };
    private static string Required(JsonElement parameters, string key) =>
        parameters.TryGetProperty(key, out var value) && value.ValueKind == JsonValueKind.String
            ? value.GetString()! : throw new ArgumentException($"Missing string parameter '{key}'.");
    private static object LoadModel(string path)
    {
        var model = TmdlSerializer.DeserializeModelFromFolder(Path.GetFullPath(path));
        return new
        {
            tables = model.Tables.Select(table => new
            {
                name = table.Name,
                columns = table.Columns.Select(column => new { name = column.Name, data_type = column.DataType.ToString() }),
                measures = table.Measures.Select(measure => new { name = measure.Name, expression = measure.Expression })
            }),
            relationships = model.Relationships.OfType<SingleColumnRelationship>().Select(r => new
            {
                from_table = r.FromTable.Name, from_column = r.FromColumn.Name,
                to_table = r.ToTable.Name, to_column = r.ToColumn.Name,
                active = r.IsActive,
                single_direction = r.CrossFilteringBehavior == CrossFilteringBehavior.OneDirection &&
                    r.FromCardinality == RelationshipEndCardinality.Many && r.ToCardinality == RelationshipEndCardinality.One
            })
        };
    }
    private object Snapshot(string table, string path)
    {
        if (!Path.IsPathFullyQualified(path)) throw new ArgumentException("Snapshot path must be absolute.");
        return Query($"EVALUATE '{table.Replace("'", "''")}'", path, false);
    }
    private object Query(string query, string? destination, bool temporary)
    {
        var path = destination ?? Path.Combine(Path.GetTempPath(), $"lightingdax-{Guid.NewGuid():N}.arrow");
        // Never replace a previous snapshot silently. Export to a new file.
        if (File.Exists(path)) throw new InvalidOperationException("Output file exists; choose a new snapshot path.");
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        var created = false;
        try
        {
            using var command = Connection.CreateCommand();
            command.CommandText = query;
            command.CommandTimeout = 30;
            using var reader = command.ExecuteReader();
            if (reader.FieldCount == 0) throw new InvalidOperationException("Query returned no columns.");
            var names = Enumerable.Range(0, reader.FieldCount).Select(reader.GetName).ToArray();
            var types = Enumerable.Range(0, reader.FieldCount).Select(i => LogicalType(reader.GetFieldType(i))).ToArray();
            var schema = new Schema(names.Select((name, i) => new Field(name, ArrowType(types[i]), true)), null);
            using var stream = new FileStream(path, FileMode.CreateNew, FileAccess.Write, FileShare.None);
            created = true;
            using var writer = new ArrowFileWriter(stream, schema);
            writer.WriteStart();
            var buffers = names.Select(_ => new List<object?>()).ToArray();
            long rows = 0;
            var maxRows = temporary ? 100_000L : long.MaxValue;
            while (reader.Read())
            {
                if (++rows > maxRows) throw new InvalidOperationException("Query exceeds the 100,000 row results limit. Add TOPN or export a table snapshot.");
                for (var i = 0; i < names.Length; i++) buffers[i].Add(reader.IsDBNull(i) ? null : reader.GetValue(i));
                if (buffers[0].Count >= 8192) WriteBatch(writer, schema, buffers, types);
            }
            if (names.Length > 0 && buffers[0].Count > 0) WriteBatch(writer, schema, buffers, types);
            writer.WriteEnd();
            return new { path, rows, types, temporary, engine_version = Connection.ServerVersion };
        }
        catch { if (created && File.Exists(path)) File.Delete(path); throw; }
    }
    private static string LogicalType(Type type)
    {
        if (type == typeof(bool)) return "boolean";
        if (type == typeof(DateTime)) return "datetime";
        if (type == typeof(decimal)) return "decimal";
        if (type == typeof(double) || type == typeof(float)) return "number";
        if (type == typeof(long) || type == typeof(int) || type == typeof(short) || type == typeof(byte)) return "integer";
        if (type == typeof(string)) return "text";
        throw new NotSupportedException($"Unsupported engine field type '{type.Name}'.");
    }
    private static IArrowType ArrowType(string type) => type switch
    {
        "boolean" => BooleanType.Default, "number" => DoubleType.Default,
        "integer" => Int64Type.Default, _ => StringType.Default
    };
    private static void WriteBatch(ArrowFileWriter writer, Schema schema, List<object?>[] columns, string[] types)
    {
        var arrays = columns.Select((values, i) => BuildArray(values, types[i])).ToArray();
        using var batch = new RecordBatch(schema, arrays, columns[0].Count);
        writer.WriteRecordBatch(batch);
        foreach (var values in columns) values.Clear();
    }
    private static IArrowArray BuildArray(List<object?> values, string type)
    {
        if (type == "integer")
        {
            var b = new Int64Array.Builder(); foreach (var v in values) { if (v is null) b.AppendNull(); else b.Append(Convert.ToInt64(v, CultureInfo.InvariantCulture)); } return b.Build();
        }
        if (type == "number")
        {
            var b = new DoubleArray.Builder(); foreach (var v in values) { if (v is null) b.AppendNull(); else b.Append(Convert.ToDouble(v, CultureInfo.InvariantCulture)); } return b.Build();
        }
        if (type == "boolean")
        {
            var b = new BooleanArray.Builder(); foreach (var v in values) { if (v is null) b.AppendNull(); else b.Append(Convert.ToBoolean(v, CultureInfo.InvariantCulture)); } return b.Build();
        }
        var strings = new StringArray.Builder();
        foreach (var v in values)
        {
            if (v is null) strings.AppendNull();
            else if (v is DateTime date) strings.Append(date.ToString("O", CultureInfo.InvariantCulture));
            else strings.Append(Convert.ToString(v, CultureInfo.InvariantCulture));
        }
        return strings.Build();
    }
    private object ListFunctions()
    {
        using var command = Connection.CreateCommand();
        command.CommandText = "SELECT * FROM $SYSTEM.MDSCHEMA_FUNCTIONS";
        command.CommandTimeout = 30;
        using var reader = command.ExecuteReader();
        var rows = new List<Dictionary<string, object?>>();
        while (reader.Read())
        {
            var row = new Dictionary<string, object?>();
            for (var i = 0; i < reader.FieldCount; i++) row[reader.GetName(i)] = reader.IsDBNull(i) ? null : reader.GetValue(i);
            rows.Add(row);
        }
        return rows;
    }
    public void Dispose() => connection?.Dispose();
}
