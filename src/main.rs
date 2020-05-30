#![allow(dead_code)]

use std::collections::HashMap;
use std::fmt;
use std::fmt::{Debug, Display};
use std::mem;
use num::{PrimInt, ToPrimitive, Unsigned};

trait Bits: Debug + PrimInt + Unsigned {
    // has to be unsigned
    //   1. using them as bytes anyway
    //   2. max_value is all ones

    fn from_u64(val: u64) -> Self {
       Self::from(val & Self::max_value().to_u64().unwrap()).unwrap()
    }

    fn mask(ct: usize) -> Self {
        !(Self::max_value() << ct)
    }

    fn is_bit_set(self, at: usize) -> bool {
        self >> at & Self::one() == Self::one()
    }

    fn eat(mut self, mut val: Self) -> Self {
        let bit_sz = mem::size_of::<Self>() * 8;
        let mut ret = Self::zero();
        for i in 0..bit_sz {
            if self & Self::one() == Self::one() {
                ret = ret | ((val & Self::one()) << i);
                val = val >> 1;
            }
            self = self >> 1;
        }
        ret
    }

    fn to_bytes(mut self, ct: usize) -> Vec<u8>
    where
        Self: ToPrimitive {
        let mut ret = Vec::new();
        ret.reserve(ct);
        for _ in 0..ct {
            ret.push((self & Self::from(0xff).unwrap()).to_u8().unwrap());
            self = self >> 8;
        }
        ret
    }
}

// impl Bits for u8 {}
// impl Bits for u16 {}
// impl Bits for u32 {}
impl Bits for u64 {}

//

#[derive(Debug)]
struct Opdef {
    base: u64,
    args: Vec<u64>,
}

impl Opdef {
    fn new(spec_str: &str, arg_order: &str) -> Self {
        let spec_bytes = spec_str.as_bytes();
        let arg_bytes = arg_order.as_bytes();

        let base = spec_bytes.iter()
            .fold(0, | acc, byte | {
                if *byte == '1' as u8 {
                    (acc << 1) | 1
                } else {
                    acc << 1
                }
            });

        let args = arg_bytes.iter()
            .map(| arg_byte | {
                spec_bytes.iter()
                    .fold(0, | acc, spec_byte | {
                        if *arg_byte == *spec_byte {
                            (acc << 1) | 1
                        } else {
                            acc << 1
                        }
                    })
            })
            .collect();

        Opdef {
            base,
            args,
        }
    }

    fn apply(&self, arg_vals: &[u64]) -> u64 {
        // TODO check arg_vals and args same len
        arg_vals.iter()
            .zip(self.args.iter())
            .map(| (&arg, &mask) | mask.eat(arg))
            .fold(self.base, | acc, x | acc | x)
    }
}

// instructions are used because
//   for jumps you need to know the location of all the opdefs
//     before you generate the code for them

#[derive(Debug)]
struct Idef {
    name: String,
    opdef: Opdef,
    shift: i32,
}

impl Idef {
    fn apply(&self, args: &[u64]) -> u64 {
        if self.shift == 0 {
            self.opdef.apply(&args)
        } else {
            let arg = args[0] >> self.shift;
            self.opdef.apply(&[arg])
        }
    }
}

//

#[derive(Copy, Clone, Debug)]
enum HexRecordType {
    Data,
    EndOfFile,
    ExtendedSegmentAddress,
    StartSegmentAddress,
    ExtendedLinearAddress,
    StarLinearAddress,
}

#[derive(Debug)]
struct HexRecord {
    typ: HexRecordType,
    addr: u16,
    data: Vec<u8>,
}

impl HexRecord {
    fn checksum(&self) -> u8 {
        fn twos_complement(num: u16) -> u16 {
            (num ^ u16::max_value()).wrapping_add(1)
        }

        let mut ret: u16 = 0;

        ret = ret.wrapping_add(self.addr);
        ret = ret.wrapping_add(self.typ as u16);
        ret = ret.wrapping_add(self.addr >> 8);
        ret = ret.wrapping_add(self.data.len() as u16);
        for byte in &self.data {
            ret = ret.wrapping_add(*byte as u16)
        }

        (0xff & twos_complement(ret)) as u8
    }
}

impl Display for HexRecord {
    fn fmt(&self, f: &mut fmt::Formatter)-> fmt::Result {
        write!(f, ":{:02x}{:04x}{:02x}"
             , self.data.len() & 0xff
             , self.addr
             , self.typ as u8)?;
        for byte in &self.data {
            write!(f, "{:02x}", *byte)?;
        }
        write!(f, "{:02x}", self.checksum())
    }
}

//

#[derive(Debug)]
enum IArg {
    Raw(u64),
    LabelAccess {
        name: String,
        is_relative: bool,
        offset: i32,
    }
}

#[derive(Debug)]
enum CodeObject<'a> {
    Instruction {
        idef: &'a Idef,
        args: Vec<IArg>,
    },
    RawData(u64),
    AddressTag(u64),
    LabelTag(String),
}

//

#[derive(Debug)]
struct CompileSettings {
    eof_record: HexRecord,
    words_per_record: u8,
}

#[derive(Debug)]
enum CompileError {
    StartWithAddressTag,
    DuplicateLabel(String),
}

#[derive(Debug)]
struct CodeInfo {
    opcode_size: u8,
    address_size: u8,
}

#[derive(Debug)]
struct Code<'a> {
    info: CodeInfo,
    code: Vec<CodeObject<'a>>,
    addr_image: Vec<u64>,
    label_table: HashMap<String, u64>,
}

// TODO check that code.len() doesnt wrapover type of A
//       for the A::from(i).unwrap()
impl<'a> Code<'a> {
    fn new(info: CodeInfo, code: Vec<CodeObject<'a>>) -> Result<Self, CompileError> {
        use CodeObject::*;

        let mut offset =
            if let AddressTag(addr) = code[0] {
                addr
            } else {
                return Err(CompileError::StartWithAddressTag);
            };

        let mut addr_image = Vec::new();
        addr_image.reserve(code.len());

        for i in 0..code.len() {
            addr_image.push(offset);
            match code[i] {
                AddressTag(addr) => {
                    offset = addr;
                },
                RawData(_) | Instruction{..} => {
                    offset += 1;
                },
                _ => {}
            }
        }

        let addr_image = addr_image;
        let mut label_table = HashMap::new();

        for (&addr, obj) in addr_image.iter().zip(code.iter()) {
            if let LabelTag(label) = obj {
                if label_table.contains_key(label) {
                    return Err(CompileError::DuplicateLabel(label.clone()));
                }
                label_table.insert(label.clone(), addr);
            }
        }

        Ok(Code {
            info,
            code,
            addr_image,
            label_table,
        })
    }
}

//

fn main() {
    // let k: u32 = 0b01010011.eat(0b0101);
    // let k = u8::mask(3);
    // let f = 0xdeadbeefu32.to_bytes(8);
    // println!("0b{:08b}", k);
    // println!("{:?}", f);
    // println!("{} {} {} {}", 0xde, 0xad, 0xbe, 0xef);

    // let opd = Opdef::<u8>::new("0101aabb", "ab");
    // println!("{:08b}", opd.apply(&[0b11, 0b01]));
    // println!("{:08b}", opd.apply(&[0b01, 0b11]));
    // println!("{:08b}", opd.apply(&[0b10, 0b00]));

    let idef_add = Idef {
        name: "add".to_string(),
        opdef: Opdef::new("0011aabb", "ab"),
        shift: 0,
    };

    let idef_jmp = Idef {
        name: "jmp".to_string(),
        opdef: Opdef::new("0011aaaa", "a"),
        shift: 8,
    };

    let c = Code::new(CodeInfo{opcode_size: 8, address_size: 16},
        vec![
            CodeObject::AddressTag(0),
            CodeObject::LabelTag("start".to_string()),
            CodeObject::Instruction{idef: &idef_add, args: vec![IArg::Raw(0b11), IArg::Raw(0b00)]},
            CodeObject::Instruction{idef: &idef_jmp, args: vec![IArg::Raw(0xdead)]},
            CodeObject::RawData(0xad),
        ]);

    println!("{:?}", c);

    let c = Code::new(CodeInfo{opcode_size: 8, address_size: 16},
        vec![
            CodeObject::AddressTag(0),
            CodeObject::LabelTag("start".to_string()),
            CodeObject::Instruction{idef: &idef_add, args: vec![IArg::Raw(0b11), IArg::Raw(0b00)]},
            CodeObject::Instruction{idef: &idef_jmp, args: vec![IArg::Raw(0xdead)]},
            CodeObject::LabelTag("another".to_string()),
            CodeObject::RawData(0xad),
        ]);

    println!("{:?}", c);

    /*
    println!("{:08b}", idef_add.apply(&[0b10, 0b11])[0]);
    let vals = idef_jmp.apply(&[0b0000100011001110]);
    println!("{:08b} {:08b}", vals[0], vals[1]);

    let hr = HexRecord {
        typ: HexRecordType::Data,
        addr: 0x0030,
        data: vec![
            0x02,
            0x33,
            0x7a,
        ]
    };
    println!("{}", hr);
    */
}
