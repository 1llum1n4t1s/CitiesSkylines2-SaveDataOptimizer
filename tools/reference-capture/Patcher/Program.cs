using System;
using System.IO;
using System.Linq;
using Colossal.Mono.Cecil;
using Colossal.Mono.Cecil.Cil;

if (args.Length != 3)
    throw new ArgumentException("original.dll capture.dll patched.dll");
if (File.Exists(args[2]))
    throw new IOException("The patched output already exists");

using var game = ModuleDefinition.ReadModule(args[0]);
using var probe = ModuleDefinition.ReadModule(args[1]);
var reader = game.GetType("Colossal.Serialization.Entities.BinaryReader");
var method = reader.Methods.Single(m => m.Name == "Read" && m.Parameters.Count == 1
    && m.Parameters[0].ParameterType.FullName == "Unity.Entities.Entity&");
var instructions = method.Body.Instructions;
var readInt = instructions.Single(i => i.OpCode == OpCodes.Call && i.Operand is MethodReference call
    && call.Name == "Read" && call.Parameters.Count == 1 && call.Parameters[0].ParameterType.FullName == "System.Int32&");
if (readInt.Previous.Operand is not VariableDefinition id)
    throw new InvalidDataException("Could not find the decoded saved ID local");
var position = reader.Fields.Single(f => f.Name == "m_Position");
var buffer = reader.Fields.Single(f => f.Name == "m_Buffer");
var getPosition = reader.Methods.Where(m => m.HasBody).SelectMany(m => m.Body.Instructions)
    .Select(i => i.Operand).OfType<MethodReference>().First(m => m.Name == "get_Value"
        && m.DeclaringType.FullName.StartsWith("Unity.Collections.NativeReference`1<System.Int32>", StringComparison.Ordinal));
var record = game.ImportReference(probe.GetType("SaveOptimizerCapture.ReferenceCapture")
    .Methods.Single(m => m.Name == "Record"));
var il = method.Body.GetILProcessor();
var next = readInt.Next;
foreach (var ins in new[] {
    il.Create(OpCodes.Ldarg_0), il.Create(OpCodes.Ldfld, buffer),
    il.Create(OpCodes.Ldarg_0), il.Create(OpCodes.Ldflda, position),
    il.Create(OpCodes.Call, getPosition), il.Create(OpCodes.Ldc_I4_4), il.Create(OpCodes.Sub),
    il.Create(OpCodes.Ldloc, id), il.Create(OpCodes.Call, record)
})
    il.InsertBefore(next, ins);

var strideRead = reader.Methods.Single(m => m.Name == "Read" && m.Parameters.Count == 2
    && m.Parameters[0].ParameterType.FullName == "Unity.Collections.NativeArray`1<System.Byte>"
    && m.Parameters[1].ParameterType.FullName == "System.Int32");
var strideRecord = game.ImportReference(probe.GetType("SaveOptimizerCapture.ReferenceCapture")
    .Methods.Single(m => m.Name == "RecordStride"));
var strideIl = strideRead.Body.GetILProcessor();
var fingerprint = game.ImportReference(probe.GetType("SaveOptimizerCapture.ReferenceCapture")
    .Methods.Single(m => m.Name == "Fingerprint"));
var before = new VariableDefinition(game.TypeSystem.UInt64);
strideRead.Body.Variables.Add(before);
var begin = strideRead.Body.Instructions[0];
foreach (var ins in new[] {
    strideIl.Create(OpCodes.Ldarg_0), strideIl.Create(OpCodes.Ldfld, buffer),
    strideIl.Create(OpCodes.Call, fingerprint), strideIl.Create(OpCodes.Stloc, before)
})
    strideIl.InsertBefore(begin, ins);
var end = strideRead.Body.Instructions.Single(i => i.OpCode == OpCodes.Ret);
foreach (var ins in new[] {
    strideIl.Create(OpCodes.Ldarg_0), strideIl.Create(OpCodes.Ldfld, buffer),
    strideIl.Create(OpCodes.Ldarg_1), strideIl.Create(OpCodes.Ldarg_0),
    strideIl.Create(OpCodes.Ldflda, position), strideIl.Create(OpCodes.Call, getPosition),
    strideIl.Create(OpCodes.Ldarg_2), strideIl.Create(OpCodes.Ldloc, before),
    strideIl.Create(OpCodes.Call, strideRecord)
})
    strideIl.InsertBefore(end, ins);

game.Write(args[2]);
Console.WriteLine("Instrumented " + method.FullName);
