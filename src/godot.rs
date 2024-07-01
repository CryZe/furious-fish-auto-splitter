#![allow(unused)]

use core::{
    any::type_name,
    fmt, iter,
    marker::PhantomData,
    mem::{size_of, MaybeUninit},
    ops::Deref,
};

use asr::{arrayvec::ArrayVec, string::ArrayString, Address, Address64, PointerSize, Process};
use bytemuck::{CheckedBitPattern, Pod, Zeroable};

#[repr(transparent)]
pub struct Ptr<T>(Address64, PhantomData<fn() -> T>);

impl<T> fmt::Debug for Ptr<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}*: {}", type_name::<T>(), self.0)
    }
}

impl<T> Copy for Ptr<T> {}
impl<T> Clone for Ptr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

unsafe impl<T: 'static> Pod for Ptr<T> {}
unsafe impl<T> Zeroable for Ptr<T> {}

impl<T> Ptr<T> {
    pub fn new(addr: Address64) -> Self {
        Self(addr, PhantomData)
    }

    pub fn deref(self, process: &Process) -> Result<T, ()>
    where
        T: CheckedBitPattern,
    {
        process.read(self.0).map_err(drop)
    }

    pub fn cast<U>(self) -> Ptr<U> {
        Ptr::new(self.0)
    }

    pub fn addr(self) -> Address64 {
        self.0
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(transparent)]
pub struct Object;

impl Ptr<Object> {
    pub fn get_type(self, process: &Process) -> Result<ObjectType, ()> {
        process.read(self.0).map_err(drop)
    }
}

#[derive(Debug, Copy, Clone, Pod, Zeroable, PartialEq, Eq)]
#[repr(transparent)]
pub struct ObjectType(Address64);

#[derive(Debug, Copy, Clone)]
#[repr(transparent)]
pub struct SceneTree;

impl SceneTree {
    pub fn get(process: &Process, module: Address) -> Result<Ptr<Self>, ()> {
        Ok(Ptr::new(process.read(module + 0x0424BE40).map_err(drop)?))
    }
}

impl Ptr<SceneTree> {
    pub fn get_root(self, process: &Process) -> Result<Ptr<Node>, ()> {
        process.read(self.0 + 0x2B0).map_err(drop).map(Ptr::new)
    }

    pub fn get_current_frame(self, process: &Process) -> Result<i64, ()> {
        process.read(self.0 + 0x330).map_err(drop)
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(transparent)]
pub struct Node;

impl Ptr<Node> {
    pub fn get_parent(self, process: &Process) -> Result<Option<Self>, ()> {
        process
            .read(self.0 + 0x128)
            .map_err(drop)
            .map(|addr: Address64| {
                if addr.is_null() {
                    None
                } else {
                    Some(Self::new(addr))
                }
            })
    }

    pub fn get_owner(self, process: &Process) -> Result<Option<Self>, ()> {
        process
            .read(self.0 + 0x130)
            .map_err(drop)
            .map(|addr: Address64| {
                if addr.is_null() {
                    None
                } else {
                    Some(Self::new(addr))
                }
            })
    }

    pub fn children(self) -> Ptr<HashMap<StringName, Ptr<Node>>> {
        Ptr::new(self.0 + 0x138)
    }

    pub fn get_name<const N: usize>(self, process: &Process) -> Result<String<N>, ()> {
        let string_name: StringName = process.read(self.0 + 0x1D0).map_err(drop)?;
        string_name.read(process)
    }
}

impl Deref for Ptr<Node> {
    type Target = Ptr<Object>;

    fn deref(&self) -> &Self::Target {
        bytemuck::cast_ref(self)
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(transparent)]
pub struct CanvasItem;

impl Ptr<CanvasItem> {
    pub fn get_global_transform(self, process: &Process) -> Result<[f32; 6], ()> {
        process.read(self.0 + 0x450).map_err(drop)
    }
}

impl Deref for Ptr<CanvasItem> {
    type Target = Ptr<Node>;

    fn deref(&self) -> &Self::Target {
        bytemuck::cast_ref(self)
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(transparent)]
pub struct Node2D;

impl Ptr<Node2D> {
    pub fn get_position(self, process: &Process) -> Result<[f32; 2], ()> {
        process.read(self.0 + 0x48C).map_err(drop)
    }

    pub fn get_rotation(self, process: &Process) -> Result<f32, ()> {
        process.read(self.0 + 0x494).map_err(drop)
    }

    pub fn get_scale(self, process: &Process) -> Result<[f32; 2], ()> {
        process.read(self.0 + 0x498).map_err(drop)
    }
}

impl Deref for Ptr<Node2D> {
    type Target = Ptr<CanvasItem>;

    fn deref(&self) -> &Self::Target {
        bytemuck::cast_ref(self)
    }
}

pub trait ProperlySized {}

#[derive(Debug, Copy, Clone)]
#[repr(transparent)]
pub struct HashMap<K, V>(core::marker::PhantomData<(K, V)>);

impl<K, V> Ptr<HashMap<K, V>> {
    pub fn iter<'a>(&'a self, process: &'a Process) -> impl Iterator<Item = (Ptr<K>, Ptr<V>)> + 'a
    where
        K: ProperlySized,
    {
        let mut current: Address64 = process.read(self.0 + 0x18).unwrap_or_default();
        iter::from_fn(move || {
            if current.is_null() {
                return None;
            }
            let ret = (
                Ptr::new(current + 0x10),
                Ptr::new(current + 0x10 + size_of::<K>() as u64),
            );
            current = process.read(current).ok()?;
            Some(ret)
        })
    }

    pub fn get_len(self, process: &Process) -> Result<u32, ()> {
        // HashMap::num_elements
        // https://github.com/godotengine/godot/blob/4ab8fb809396fa38ba929fec97cfcb7193f1c44d/core/templates/hash_map.h#L82
        process.read(self.0 + 0x2C).map_err(drop)
    }
}

#[derive(Debug, Copy, Clone, Pod, Zeroable)]
#[repr(transparent)]
pub struct StringName(Ptr<StringNameData>);

impl ProperlySized for StringName {}

#[derive(Debug, Copy, Clone, Pod, Zeroable)]
#[repr(transparent)]
struct StringNameData(Address64);

impl StringName {
    pub fn read<const N: usize>(self, process: &Process) -> Result<String<N>, ()> {
        let cow_data: Address64 = process.read(self.0 .0 + 0x10).map_err(drop)?;

        // Only on 4.2 or before.
        let len = process.read::<u32>(cow_data + -0x4).map_err(drop)? - 1;
        let mut buf = [MaybeUninit::uninit(); N];
        let buf = buf.get_mut(..len as usize).ok_or(())?;
        let buf = process
            .read_into_uninit_slice(cow_data, buf)
            .map_err(drop)?;

        let mut out = ArrayVec::new();
        out.extend(buf.iter().copied());

        Ok(String(out))
    }
}

#[derive(Clone)]
pub struct String<const N: usize>(ArrayVec<u32, N>);

impl<const N: usize> String<N> {
    pub fn chars(&self) -> impl Iterator<Item = char> + '_ {
        self.0
            .iter()
            .copied()
            .map(|c| char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER))
    }

    pub fn to_array_string<const UTF8_SIZE: usize>(&self) -> ArrayString<UTF8_SIZE> {
        let mut buf = ArrayString::<UTF8_SIZE>::new();
        for c in self.chars() {
            buf.push(c);
        }
        buf
    }

    pub fn matches_str(&self, text: &str) -> bool {
        self.chars().eq(text.chars())
    }
}
