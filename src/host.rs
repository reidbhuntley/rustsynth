use std::{any::Any, fmt::Display};
use thiserror::Error;

use arr_macro::arr;
use rodio::Source;

use crate::{constants::*, midi::MidiEvents, output::AudioOutput, output::AudioOutputModule};

use self::private::{BufferInPort, FastHashMap, ModuleBuffersDescriptor, ModuleInternals};

pub trait BufferElem: 'static + private::BufferElemSealed + Default + Clone {
    fn new_buffer(self) -> Buffer<Self> {
        arr![self.clone(); 512]
    }

    fn new_vec<T: BufferElem>(len: usize) -> Vec<T> {
        std::iter::repeat(T::default()).take(len).collect()
    }
}
impl BufferElem for f32 {}
impl BufferElem for MidiEvents {}

pub trait BufferDir: private::BufferDirSealed {}
impl<T: BufferElem> BufferDir for In<T> {}
impl<T: BufferElem> BufferDir for Out<T> {}

mod private {
    use std::{collections::HashMap, sync::atomic::AtomicUsize};

    use seahash::SeaHasher;

    use super::{
        BufferArity, BufferDir, BufferDirEnum, BufferElem, BufferElemEnum, BufferHandle,
        BufferHandleRaw, BufferType, BuffersInExt, BuffersOutExt, HostError, HostIdentifier,
        HostResult, In, Module, ModuleBufferHandle, ModuleBuffersIn, ModuleBuffersOut,
        ModuleDescriptor, ModuleError, ModuleResult, ModuleSettings, Out, VariadicBufferHandle,
    };

    #[derive(Clone, Default)]
    pub struct BuildHasher;

    impl std::hash::BuildHasher for BuildHasher {
        type Hasher = SeaHasher;

        #[inline(always)]
        fn build_hasher(&self) -> Self::Hasher {
            SeaHasher::new()
        }
    }

    pub type FastHashMap<K, V> = HashMap<K, V, BuildHasher>;

    #[derive(Clone)]
    pub struct BufferOutPort<T: BufferElem> {
        pub buffer: super::Buffer<T>,
        pub dependents: Vec<ModuleBufferHandle<In<T>>>,
    }

    #[derive(Clone)]
    pub enum BufferInPort<T: BufferElem> {
        OutBuffer(ModuleBufferHandle<Out<T>>),
        Constant(super::Buffer<T>),
    }

    impl<T: BufferElem> BufferInPort<T> {
        pub fn with_constant(value: T) -> Self {
            Self::Constant(T::new_buffer(value))
        }
    }

    impl<T: BufferElem> Default for BufferInPort<T> {
        fn default() -> Self {
            Self::with_constant(T::default())
        }
    }

    enum HandleArity<T: BufferDir> {
        Single(BufferHandle<T>),
        Variadic(VariadicBufferHandle<T>),
    }

    pub struct BufferPorts<D: BufferDir> {
        num_args: usize,
        pub buffers: Vec<D::BufferPort>,
        handles: FastHashMap<String, HandleArity<D>>,
    }

    impl<D: BufferDir> BufferPorts<D> {
        fn new(descriptor: &ModuleDescriptor) -> ModuleResult<Self> {
            let mut out = Self {
                num_args: descriptor.num_args,
                buffers: Default::default(),
                handles: Default::default(),
            };
            for (marker, name, elem) in D::get_elems(descriptor).iter() {
                out.add_buffer(*marker, D::create_port(elem), name)?;
            }
            Ok(out)
        }

        pub fn get_buf(&self, handle: BufferHandle<D>) -> &D::BufferPort {
            &self.buffers[handle.idx]
        }

        pub fn get_buf_mut(&mut self, handle: BufferHandle<D>) -> &mut D::BufferPort {
            &mut self.buffers[handle.idx]
        }

        pub fn get_handle(&self, name: &str) -> HostResult<BufferHandle<D>> {
            match self.handles.get(name) {
                Some(HandleArity::Single(handle)) => Ok(*handle),
                Some(HandleArity::Variadic(_)) => Err(HostError::UnexpectedBufferArity {
                    expected: BufferArity::Single,
                    found: BufferArity::Variadic,
                }),
                None => Err(HostError::NonexistentIdentifier {
                    ident: name.to_owned(),
                    ident_type: HostIdentifier::Buffer(D::name()),
                }),
            }
        }

        pub fn get_variadic_handle(&self, name: &str) -> HostResult<VariadicBufferHandle<D>> {
            match self.handles.get(name) {
                Some(HandleArity::Single(_)) => Err(HostError::UnexpectedBufferArity {
                    expected: BufferArity::Single,
                    found: BufferArity::Variadic,
                }),
                Some(HandleArity::Variadic(handle)) => Ok(*handle),
                None => Err(HostError::NonexistentIdentifier {
                    ident: name.to_owned(),
                    ident_type: HostIdentifier::Buffer(D::name()),
                }),
            }
        }

        fn add_buffer(
            &mut self,
            marker: BufferArity,
            port: D::BufferPort,
            name: &str,
        ) -> ModuleResult<()> {
            if self.handles.contains_key(name) {
                Err(ModuleError::DuplicateBufferIdentifier {
                    ident: name.to_owned(),
                    buffer_type: D::name(),
                })
            } else {
                let handle = BufferHandle::new(self.buffers.len());
                self.handles.insert(
                    name.to_owned(),
                    match marker {
                        BufferArity::Single => {
                            self.buffers.push(port.clone());
                            HandleArity::Single(handle)
                        }
                        BufferArity::Variadic => {
                            for _ in 0..self.num_args {
                                self.buffers.push(port.clone());
                            }
                            HandleArity::Variadic(VariadicBufferHandle {
                                num_args: self.num_args,
                                buffer: handle,
                            })
                        }
                    },
                );
                Ok(())
            }
        }
    }

    #[derive(Clone)]
    pub struct ModuleBuffersDescriptor<T: BufferElem> {
        num_args: usize,
        next_idx_buf_in: BufferHandleRaw,
        next_idx_buf_out: BufferHandleRaw,
        pub buf_in: Vec<(BufferArity, String, T)>,
        pub buf_out: Vec<(BufferArity, String, ())>,
    }

    impl<T: BufferElem> ModuleBuffersDescriptor<T> {
        pub fn new(num_args: usize) -> Self {
            Self {
                num_args,
                next_idx_buf_in: 0,
                next_idx_buf_out: 0,
                buf_in: Default::default(),
                buf_out: Default::default(),
            }
        }

        pub fn add_buf_in(&mut self, elem: (BufferArity, String, T)) -> BufferHandleRaw {
            let out = self.next_idx_buf_in;
            self.next_idx_buf_in += match elem.0 {
                BufferArity::Single => 1,
                BufferArity::Variadic => self.num_args,
            };
            self.buf_in.push(elem);
            out
        }

        pub fn add_buf_out(&mut self, elem: (BufferArity, String, ())) -> BufferHandleRaw {
            let out = self.next_idx_buf_out;
            self.next_idx_buf_out += match elem.0 {
                BufferArity::Single => 1,
                BufferArity::Variadic => self.num_args,
            };
            self.buf_out.push(elem);
            out
        }
    }

    pub struct ModuleBuffersInInternal {
        pub num_dependencies: usize,
        pub num_finished_dependencies: AtomicUsize,
        buf_signal: BufferPorts<In<f32>>,
        buf_midi: BufferPorts<In<crate::midi::MidiEvents>>,
    }

    impl ModuleBuffersInInternal {
        pub fn new(descriptors: &ModuleDescriptor) -> ModuleResult<Self> {
            Ok(Self {
                num_dependencies: 0,
                num_finished_dependencies: AtomicUsize::new(0),
                buf_signal: BufferPorts::new(&descriptors)?,
                buf_midi: BufferPorts::new(&descriptors)?,
            })
        }
    }

    pub struct ModuleBuffersOutInternal {
        buf_signal: BufferPorts<Out<f32>>,
        buf_midi: BufferPorts<Out<crate::midi::MidiEvents>>,
    }

    impl ModuleBuffersOutInternal {
        pub fn new(descriptors: &ModuleDescriptor) -> ModuleResult<Self> {
            Ok(Self {
                buf_signal: BufferPorts::new(&descriptors)?,
                buf_midi: BufferPorts::new(&descriptors)?,
            })
        }
    }

    pub struct ModuleInternals {
        pub module: Box<dyn Module>,
        pub num_args: usize,
        pub buf_in: ModuleBuffersInInternal,
        pub buf_out: ModuleBuffersOutInternal,
    }

    impl ModuleInternals {
        pub fn new<T: Module + ModuleSettings>(
            settings: T::Settings,
            num_args: usize,
        ) -> ModuleResult<Self> {
            let descriptor = T::init(ModuleDescriptor::new(num_args), settings, num_args)
                .map_err(|e| ModuleError::Custom(e.to_string()))?;
            Ok(Self {
                module: descriptor.initial_data,
                num_args,
                buf_in: ModuleBuffersInInternal::new(&descriptor.buffers_descriptors)?,
                buf_out: ModuleBuffersOutInternal::new(&descriptor.buffers_descriptors)?,
            })
        }
    }

    pub trait BufferDirSealed {
        type DescriptorElem;
        type BufferPort: Clone;
        fn name() -> BufferType;
        fn get_buffers(internals: &ModuleInternals) -> &BufferPorts<Self>
        where
            Self: Sized + BufferDir;
        fn get_elems(
            descriptor: &ModuleDescriptor,
        ) -> &Vec<(BufferArity, String, Self::DescriptorElem)>;
        fn create_port(elem: &Self::DescriptorElem) -> Self::BufferPort;
    }

    impl<T: BufferElem> BufferDirSealed for In<T> {
        type DescriptorElem = T;
        type BufferPort = BufferInPort<T>;

        fn name() -> BufferType {
            BufferType {
                dir: BufferDirEnum::In,
                elem: T::name(),
            }
        }

        fn get_buffers(internals: &ModuleInternals) -> &BufferPorts<Self> {
            T::get_buffers_in(&internals.buf_in)
        }

        fn get_elems(
            descriptor: &ModuleDescriptor,
        ) -> &Vec<(BufferArity, String, Self::DescriptorElem)> {
            &T::get_descriptor(descriptor).buf_in
        }

        fn create_port(elem: &Self::DescriptorElem) -> Self::BufferPort {
            BufferInPort::with_constant(elem.clone())
        }
    }

    impl<T: BufferElem> BufferDirSealed for Out<T> {
        type DescriptorElem = ();
        type BufferPort = BufferOutPort<T>;

        fn name() -> BufferType {
            BufferType {
                dir: BufferDirEnum::Out,
                elem: T::name(),
            }
        }

        fn get_buffers(internals: &ModuleInternals) -> &BufferPorts<Self> {
            T::get_buffers_out(&internals.buf_out)
        }

        fn get_elems(
            descriptor: &ModuleDescriptor,
        ) -> &Vec<(BufferArity, String, Self::DescriptorElem)> {
            &T::get_descriptor(descriptor).buf_out
        }

        fn create_port(_elem: &Self::DescriptorElem) -> Self::BufferPort {
            BufferOutPort {
                buffer: T::new_buffer(T::default()),
                dependents: Vec::new(),
            }
        }
    }

    pub trait BufferElemSealed {
        fn name() -> BufferElemEnum;

        fn get_buffers_in(buf_in: &ModuleBuffersInInternal) -> &BufferPorts<In<Self>>
        where
            Self: Sized + BufferElem;

        fn get_buffers_in_mut(buf_in: &mut ModuleBuffersInInternal) -> &mut BufferPorts<In<Self>>
        where
            Self: Sized + BufferElem;

        fn get_ext_buffers_in(buf_in: &ModuleBuffersIn) -> &BuffersInExt<Self>
        where
            Self: Sized + BufferElem;

        fn get_buffers_out(buf_out: &ModuleBuffersOutInternal) -> &BufferPorts<Out<Self>>
        where
            Self: Sized + BufferElem;

        fn get_buffers_out_mut(
            buf_out: &mut ModuleBuffersOutInternal,
        ) -> &mut BufferPorts<Out<Self>>
        where
            Self: Sized + BufferElem;

        fn get_ext_buffers_out(buf_out: &ModuleBuffersOut) -> &BuffersOutExt<Self>
        where
            Self: Sized + BufferElem;

        fn get_descriptor(descriptors: &ModuleDescriptor) -> &ModuleBuffersDescriptor<Self>
        where
            Self: Sized + BufferElem;

        fn get_descriptor_mut(
            descriptors: &mut ModuleDescriptor,
        ) -> &mut ModuleBuffersDescriptor<Self>
        where
            Self: Sized + BufferElem;
    }
    impl BufferElemSealed for f32 {
        fn name() -> BufferElemEnum {
            BufferElemEnum::Signal
        }

        fn get_buffers_in(buf_in: &ModuleBuffersInInternal) -> &BufferPorts<In<Self>> {
            &buf_in.buf_signal
        }

        fn get_buffers_in_mut(buf_in: &mut ModuleBuffersInInternal) -> &mut BufferPorts<In<Self>> {
            &mut buf_in.buf_signal
        }

        fn get_ext_buffers_in(buf_in: &ModuleBuffersIn) -> &BuffersInExt<Self> {
            &buf_in.buf_signal
        }

        fn get_buffers_out(buf_out: &ModuleBuffersOutInternal) -> &BufferPorts<Out<Self>> {
            &buf_out.buf_signal
        }

        fn get_buffers_out_mut(
            buf_out: &mut ModuleBuffersOutInternal,
        ) -> &mut BufferPorts<Out<Self>> {
            &mut buf_out.buf_signal
        }

        fn get_ext_buffers_out(buf_out: &ModuleBuffersOut) -> &BuffersOutExt<Self> {
            &buf_out.buf_signal
        }

        fn get_descriptor_mut(
            descriptors: &mut ModuleDescriptor,
        ) -> &mut ModuleBuffersDescriptor<Self> {
            &mut descriptors.buf_signal
        }

        fn get_descriptor(descriptors: &ModuleDescriptor) -> &ModuleBuffersDescriptor<Self> {
            &descriptors.buf_signal
        }
    }
    impl BufferElemSealed for crate::midi::MidiEvents {
        fn name() -> BufferElemEnum {
            BufferElemEnum::Midi
        }

        fn get_buffers_in(buf_in: &ModuleBuffersInInternal) -> &BufferPorts<In<Self>> {
            &buf_in.buf_midi
        }

        fn get_buffers_in_mut(buf_in: &mut ModuleBuffersInInternal) -> &mut BufferPorts<In<Self>> {
            &mut buf_in.buf_midi
        }

        fn get_ext_buffers_in(buf_in: &ModuleBuffersIn) -> &BuffersInExt<Self> {
            &buf_in.buf_midi
        }

        fn get_buffers_out(buf_out: &ModuleBuffersOutInternal) -> &BufferPorts<Out<Self>> {
            &buf_out.buf_midi
        }

        fn get_buffers_out_mut(
            buf_out: &mut ModuleBuffersOutInternal,
        ) -> &mut BufferPorts<Out<Self>> {
            &mut buf_out.buf_midi
        }

        fn get_ext_buffers_out(buf_out: &ModuleBuffersOut) -> &BuffersOutExt<Self> {
            &buf_out.buf_midi
        }

        fn get_descriptor_mut(
            descriptors: &mut ModuleDescriptor,
        ) -> &mut ModuleBuffersDescriptor<Self> {
            &mut descriptors.buf_midi
        }

        fn get_descriptor(descriptors: &ModuleDescriptor) -> &ModuleBuffersDescriptor<Self> {
            &descriptors.buf_midi
        }
    }
}

pub type Buffer<T> = [T; BUFFER_LEN];

type BufferHandleRaw = usize;

#[derive(Educe, Eq)]
#[educe(Clone, Copy, PartialEq)]
pub struct In<T: BufferElem>(std::marker::PhantomData<T>);

#[derive(Educe, Eq)]
#[educe(Clone, Copy, PartialEq)]
pub struct Out<T: BufferElem>(std::marker::PhantomData<T>);

#[derive(Educe, Eq)]
#[educe(Clone, Copy, PartialEq)]
pub struct BufferHandle<T: BufferDir> {
    _marker: std::marker::PhantomData<T>,
    idx: BufferHandleRaw,
}

impl<T: BufferDir> BufferHandle<T> {
    fn new(idx: BufferHandleRaw) -> Self {
        Self {
            idx,
            _marker: Default::default(),
        }
    }
}

#[derive(Educe, Eq)]
#[educe(Clone, Copy, PartialEq)]
pub struct VariadicBufferHandle<T: BufferDir> {
    num_args: usize,
    buffer: BufferHandle<T>,
}

impl<T: BufferDir> VariadicBufferHandle<T> {
    pub fn at(&self, idx: usize) -> HostResult<BufferHandle<T>> {
        if idx >= self.num_args {
            Err(HostError::VariadicBufferOutOfBounds {
                idx,
                len: self.num_args,
            })
        } else {
            Ok(BufferHandle::<T>::new(self.buffer.idx + idx))
        }
    }

    pub fn all(&self) -> impl Iterator<Item = BufferHandle<T>> + '_ {
        (0..self.num_args).map(move |i| self.at(i).unwrap())
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct ModuleHandle {
    pub idx: usize,
}

#[derive(Educe, Eq)]
#[educe(Clone, Copy, PartialEq)]
pub struct ModuleBufferHandle<T: BufferDir> {
    pub module_handle: ModuleHandle,
    pub buf_handle: BufferHandle<T>,
}

#[derive(Educe, Eq)]
#[educe(Clone, Copy, PartialEq)]
pub struct ModuleVariadicBufferHandle<T: BufferDir> {
    pub module_handle: ModuleHandle,
    pub buf_handle: VariadicBufferHandle<T>,
}

impl<T: BufferDir> ModuleVariadicBufferHandle<T> {
    pub fn at(&self, idx: usize) -> HostResult<ModuleBufferHandle<T>> {
        Ok(ModuleBufferHandle {
            module_handle: self.module_handle,
            buf_handle: self.buf_handle.at(idx)?,
        })
    }

    pub fn all(&self, group: GroupHandle) -> GroupBufferHandle<T> {
        GroupBufferHandle {
            group,
            handles: (0..self.buf_handle.num_args)
                .map(|i| self.at(i).unwrap())
                .collect(),
        }
    }
}

pub struct ModuleDescriptor {
    num_args: usize,
    buf_signal: ModuleBuffersDescriptor<f32>,
    buf_midi: ModuleBuffersDescriptor<MidiEvents>,
}

pub struct BuiltModuleDescriptor<T: Module> {
    initial_data: Box<T>,
    buffers_descriptors: ModuleDescriptor,
}

impl ModuleDescriptor {
    fn new(num_args: usize) -> Self {
        Self {
            num_args,
            buf_signal: ModuleBuffersDescriptor::new(num_args),
            buf_midi: ModuleBuffersDescriptor::new(num_args),
        }
    }

    pub fn build<T: Module>(self, initial_data: T) -> BuiltModuleDescriptor<T> {
        BuiltModuleDescriptor {
            initial_data: Box::new(initial_data),
            buffers_descriptors: self,
        }
    }

    pub fn with_buf_in_default<E: BufferElem>(
        &mut self,
        name: &str,
        default: E,
    ) -> BufferHandle<In<E>> {
        BufferHandle::new(E::get_descriptor_mut(self).add_buf_in((
            BufferArity::Single,
            name.to_owned(),
            default,
        )))
    }

    pub fn with_buf_in<E: BufferElem>(&mut self, name: &str) -> BufferHandle<In<E>> {
        self.with_buf_in_default(name, E::default())
    }

    pub fn with_buf_out<E: BufferElem>(&mut self, name: &str) -> BufferHandle<Out<E>> {
        BufferHandle::new(E::get_descriptor_mut(self).add_buf_out((
            BufferArity::Single,
            name.to_owned(),
            (),
        )))
    }

    pub fn with_variadic_buf_in_default<E: BufferElem>(
        &mut self,
        name: &str,
        default: E,
    ) -> VariadicBufferHandle<In<E>> {
        VariadicBufferHandle {
            num_args: self.num_args,
            buffer: BufferHandle::new(E::get_descriptor_mut(self).add_buf_in((
                BufferArity::Variadic,
                name.to_owned(),
                default,
            ))),
        }
    }

    pub fn with_variadic_buf_in<E: BufferElem>(
        &mut self,
        name: &str,
    ) -> VariadicBufferHandle<In<E>> {
        self.with_variadic_buf_in_default(name, E::default())
    }

    pub fn with_variadic_buf_out<E: BufferElem>(
        &mut self,
        name: &str,
    ) -> VariadicBufferHandle<Out<E>> {
        VariadicBufferHandle {
            num_args: self.num_args,
            buffer: BufferHandle::new(E::get_descriptor_mut(self).add_buf_out((
                BufferArity::Variadic,
                name.to_owned(),
                (),
            ))),
        }
    }
}

type BuffersInExt<T> = Vec<*const Buffer<T>>;

pub struct ModuleBuffersIn {
    buf_signal: BuffersInExt<f32>,
    buf_midi: BuffersInExt<MidiEvents>,
}

impl ModuleBuffersIn {
    pub fn get<T: BufferElem>(&self, handle: BufferHandle<In<T>>) -> &Buffer<T> {
        let bufs = T::get_ext_buffers_in(&self)[handle.idx];
        unsafe { &*bufs }
    }

    pub fn get_variadic<T: BufferElem>(
        &self,
        handle: VariadicBufferHandle<In<T>>,
    ) -> impl Iterator<Item = &Buffer<T>> + '_ {
        T::get_ext_buffers_in(self)
            .iter()
            .skip(handle.buffer.idx)
            .take(handle.num_args)
            .map(|&bufs| unsafe { &*bufs })
    }
}

type BuffersOutExt<T> = Vec<*mut Buffer<T>>;

pub struct ModuleBuffersOut {
    buf_signal: Vec<*mut Buffer<f32>>,
    buf_midi: Vec<*mut Buffer<MidiEvents>>,
}

impl ModuleBuffersOut {
    pub fn get<T: BufferElem>(&mut self, handle: BufferHandle<Out<T>>) -> &mut Buffer<T> {
        let bufs = T::get_ext_buffers_out(&self)[handle.idx];
        unsafe { &mut *bufs }
    }

    pub fn get_iter<T: BufferElem>(
        &mut self,
        handle: VariadicBufferHandle<Out<T>>,
    ) -> impl Iterator<Item = &mut Buffer<T>> + '_ {
        T::get_ext_buffers_out(self)
            .iter()
            .skip(handle.buffer.idx)
            .take(handle.num_args)
            .map(|&bufs| unsafe { &mut *bufs })
    }
}

pub trait ModuleSettings {
    type Settings: Clone;
    type Error: std::error::Error;
}

pub trait Module: 'static + Any {
    fn init(
        descriptor: ModuleDescriptor,
        settings: Self::Settings,
        num_variadic_args: usize,
    ) -> Result<BuiltModuleDescriptor<Self>, Self::Error>
    where
        Self: Sized + ModuleSettings;
    fn fill_buffers(&mut self, buffers_in: &ModuleBuffersIn, buffers_out: &mut ModuleBuffersOut);
}

pub struct Host {
    modules: FastHashMap<usize, ModuleInternals>,
    module_handles: FastHashMap<String, ModuleHandle>,
    next_module_idx: usize,
    groups: FastHashMap<usize, Group>,
    group_handles: FastHashMap<String, GroupHandle>,
    next_group_idx: usize,
    output: rodio::source::Stoppable<AudioOutput>,
    output_handle: ModuleHandle,
}

const OUTPUT_MODULE_NAME: &str = "audio_out";

impl Host {
    pub fn new() -> HostResult<Self> {
        let output = AudioOutput::new();
        let mut out = Self {
            modules: Default::default(),
            module_handles: Default::default(),
            next_module_idx: 0,
            groups: Default::default(),
            group_handles: Default::default(),
            next_group_idx: 0,
            output: output.clone().stoppable(),
            output_handle: ModuleHandle { idx: 0 },
        };
        out.output_handle = out.create_module::<AudioOutputModule>(OUTPUT_MODULE_NAME, output)?;
        Ok(out)
    }

    pub fn get_output_module(&self) -> ModuleHandle {
        self.output_handle
    }

    fn create_variadic_module_anonymous<T: Module + ModuleSettings>(
        &mut self,
        settings: T::Settings,
        num_args: usize,
    ) -> ModuleResult<ModuleHandle> {
        let module = ModuleInternals::new::<T>(settings, num_args)?;
        let idx = self.next_module_idx;
        self.next_module_idx += 1;
        self.modules.insert(idx, module);
        Ok(ModuleHandle { idx })
    }

    pub fn create_variadic_module<T: Module + ModuleSettings>(
        &mut self,
        name: &str,
        settings: T::Settings,
        num_args: usize,
    ) -> HostResult<ModuleHandle> {
        if self.module_handles.contains_key(name) {
            Err(HostError::DuplicateIdentifier {
                ident: name.to_owned(),
                ident_type: HostIdentifier::Module,
            })
        } else {
            let handle = self
                .create_variadic_module_anonymous::<T>(settings, num_args)
                .map_err(|e| HostError::ModuleInit {
                    module_name: name.to_owned(),
                    source: e,
                })?;
            self.module_handles.insert(name.to_owned(), handle);
            Ok(handle)
        }
    }

    pub fn create_module<T: Module + ModuleSettings>(
        &mut self,
        name: &str,
        settings: T::Settings,
    ) -> HostResult<ModuleHandle> {
        self.create_variadic_module::<T>(name, settings, 0)
    }

    pub fn buf<T: BufferDir>(
        &self,
        handle: ModuleHandle,
        name: &str,
    ) -> HostResult<ModuleBufferHandle<T>> {
        Ok(ModuleBufferHandle {
            module_handle: handle,
            buf_handle: T::get_buffers(&self.modules[&handle.idx]).get_handle(name)?,
        })
    }

    pub fn variadic_buf<T: BufferDir>(
        &self,
        handle: ModuleHandle,
        name: &str,
    ) -> HostResult<ModuleVariadicBufferHandle<T>> {
        Ok(ModuleVariadicBufferHandle {
            module_handle: handle,
            buf_handle: T::get_buffers(&self.modules[&handle.idx]).get_variadic_handle(name)?,
        })
    }

    fn set_buffer_in<T: BufferElem>(
        &mut self,
        port_handle: ModuleBufferHandle<In<T>>,
        new: BufferInPort<T>,
    ) {
        let module_in = self
            .modules
            .get_mut(&port_handle.module_handle.idx)
            .unwrap();

        let port = T::get_buffers_in(&module_in.buf_in).get_buf(port_handle.buf_handle);
        let (old_out, new_out) = match port {
            BufferInPort::OutBuffer(out) => (
                Some(out.clone()),
                match &new {
                    BufferInPort::Constant(_) => {
                        module_in.buf_in.num_dependencies -= 1;
                        None
                    }
                    BufferInPort::OutBuffer(new_out) => Some(new_out.clone()),
                },
            ),
            BufferInPort::Constant(_) => (
                None,
                match &new {
                    BufferInPort::Constant(_) => None,
                    BufferInPort::OutBuffer(new_out) => {
                        module_in.buf_in.num_dependencies += 1;
                        Some(new_out.clone())
                    }
                },
            ),
        };

        *T::get_buffers_in_mut(&mut module_in.buf_in).get_buf_mut(port_handle.buf_handle) = new;

        if let Some(old_out) = old_out {
            let module_out = self.modules.get_mut(&old_out.module_handle.idx).unwrap();
            T::get_buffers_out_mut(&mut module_out.buf_out)
                .get_buf_mut(old_out.buf_handle)
                .dependents
                .retain(|d| d != &port_handle);
        }

        if let Some(new_out) = new_out {
            let module_out = self.modules.get_mut(&new_out.module_handle.idx).unwrap();
            T::get_buffers_out_mut(&mut module_out.buf_out)
                .get_buf_mut(new_out.buf_handle)
                .dependents
                .push(port_handle);
        }
    }

    pub fn link<T: BufferElem>(
        &mut self,
        buf_out: ModuleBufferHandle<Out<T>>,
        buf_in: ModuleBufferHandle<In<T>>,
    ) {
        self.set_buffer_in(buf_in, BufferInPort::OutBuffer(buf_out));
    }

    pub fn link_value<T: BufferElem>(&mut self, value: T, buf_in: ModuleBufferHandle<In<T>>) {
        self.set_buffer_in(buf_in, BufferInPort::with_constant(value));
    }

    pub fn link_group<T: BufferElem>(
        &mut self,
        buf_out: &GroupBufferHandle<Out<T>>,
        buf_in: &GroupBufferHandle<In<T>>,
    ) -> HostResult<()> {
        if buf_out.group != buf_in.group {
            return Err(HostError::BufferGroupMismatch);
        }
        for (&handle_out, &handle_in) in buf_out.handles.iter().zip(buf_in.handles.iter()) {
            self.link(handle_out, handle_in);
        }
        Ok(())
    }

    pub fn link_group_ext<T: BufferElem>(
        &mut self,
        buf_out: ModuleBufferHandle<Out<T>>,
        buf_in: &GroupBufferHandle<In<T>>,
    ) {
        for &handle_in in buf_in.handles.iter() {
            self.link(buf_out, handle_in);
        }
    }

    pub fn link_group_value<T: BufferElem>(&mut self, value: T, buf_in: &GroupBufferHandle<In<T>>) {
        for &handle_in in buf_in.handles.iter() {
            self.link_value(value.clone(), handle_in);
        }
    }

    // pub fn destroy_module(&mut self, handle: ModuleHandle) {
    //     fn remove_dependents<T: BufferElem>(host: &mut Host, module: &ModuleInternals) {
    //         for out_port in T::get_buffers_out(&module.buf_out).buffers.iter() {
    //             for dep_handle in out_port.dependents.iter() {
    //                 host.set_buffer_in(dep_handle.clone(), BufferInPort::default());
    //             }
    //         }
    //     }

    //     fn remove_dependencies<T: BufferElem>(
    //         host: &mut Host,
    //         module: &ModuleInternals,
    //         module_handle: ModuleHandle,
    //     ) {
    //         for (port_idx, in_port) in T::get_buffers_in(&module.buf_in).buffers.iter().enumerate()
    //         {
    //             match in_port {
    //                 BufferInPort::OutBuffer(_) => {
    //                     host.set_buffer_in(
    //                         ModuleBufferHandle {
    //                             module_handle: module_handle.clone(),
    //                             buf_handle: BufferHandle::new(port_idx),
    //                         },
    //                         BufferInPort::<T>::default(),
    //                     );
    //                 }
    //                 BufferInPort::Constant(_) => {}
    //             }
    //         }
    //     }

    //     if handle == self.output_handle {
    //         panic!()
    //     }

    //     self.module_handles.retain(|_, &mut v| v != handle);

    //     let module = self.modules.remove(&handle.idx).unwrap();
    //     remove_dependents::<f32>(self, &module);
    //     remove_dependencies::<f32>(self, &module, handle);
    //     remove_dependents::<MidiEvents>(self, &module);
    //     remove_dependencies::<MidiEvents>(self, &module, handle);
    // }

    // pub fn update_module<T: Module + ModuleTypes>(
    //     &mut self,
    //     handle: &ModuleHandle,
    //     settings: T::Settings,
    // ) {
    //     if let Some(module) = self.modules.get_mut(&handle.idx) {
    //         assert!(module.module.as_ref().type_id() == TypeId::of::<T>());
    //         let descriptor = T::init(settings);
    //         module.module = descriptor.initial_data;
    //         // module.buf_in.set_num_buffers_all(&descriptor.buf_in);
    //         // module.buf_out.set_num_buffers_all(&descriptor.buf_out);
    //     }
    // }

    pub fn process(&mut self) -> ! {
        let (_stream, stream_handle) = rodio::OutputStream::try_default().unwrap();
        stream_handle
            .play_raw(self.output.clone().stoppable())
            .unwrap();

        loop {
            for module in self.modules.values_mut() {
                *module.buf_in.num_finished_dependencies.get_mut() = 0;
            }

            let zero_dependency_mods = self
                .modules
                .iter()
                .filter_map(|(&idx, module)| {
                    if module.buf_in.num_dependencies == 0 {
                        Some(ModuleHandle { idx })
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();

            for handle in zero_dependency_mods {
                unsafe { self.process_module(handle) };
            }
        }
    }

    unsafe fn process_module(&mut self, handle: ModuleHandle) {
        fn get_linked_ports<T: BufferElem>(
            host: &Host,
            module: &ModuleInternals,
        ) -> Vec<*const Buffer<T>> {
            T::get_buffers_in(&module.buf_in)
                .buffers
                .iter()
                .map(|port| match port {
                    BufferInPort::OutBuffer(handle) => {
                        let buf_out = &host.modules[&handle.module_handle.idx].buf_out;
                        &T::get_buffers_out(buf_out)
                            .get_buf(handle.buf_handle)
                            .buffer as *const _
                    }
                    BufferInPort::Constant(buf) => buf,
                })
                .collect::<Vec<_>>()
        }

        fn get_out_buffers<T: BufferElem>(module: &mut ModuleInternals) -> Vec<*mut Buffer<T>> {
            T::get_buffers_out_mut(&mut module.buf_out)
                .buffers
                .iter_mut()
                .map(|buf| &mut buf.buffer as *mut _)
                .collect()
        }

        fn get_dependents<T: BufferElem>(
            module: &ModuleInternals,
        ) -> impl Iterator<Item = ModuleHandle> + '_ {
            T::get_buffers_out(&module.buf_out)
                .buffers
                .iter()
                .flat_map(|buf| &buf.dependents)
                .map(|h| h.module_handle.clone())
        }

        let module = self.modules.get_mut(&handle.idx).unwrap() as *mut ModuleInternals;

        let module_ref = &*module;

        if module_ref
            .buf_in
            .num_finished_dependencies
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1
            < module_ref.buf_in.num_dependencies
        {
            return;
        }

        let buf_in = ModuleBuffersIn {
            buf_signal: get_linked_ports(&self, module_ref),
            buf_midi: get_linked_ports(&self, module_ref),
        };

        let module_mut = &mut *module;
        let mut buf_out = ModuleBuffersOut {
            buf_signal: get_out_buffers(module_mut),
            buf_midi: get_out_buffers(module_mut),
        };
        module_mut.module.fill_buffers(&buf_in, &mut buf_out);

        for dependent in
            get_dependents::<f32>(module_mut).chain(get_dependents::<MidiEvents>(module_mut))
        {
            self.process_module(dependent);
        }
    }

    pub fn create_group(
        &mut self,
        name: &str,
        anonymous_instances: usize,
        named_instances: Option<&Vec<&str>>,
    ) -> HostResult<GroupHandle> {
        if self.group_handles.contains_key(name) {
            return Err(HostError::DuplicateIdentifier {
                ident: name.to_owned(),
                ident_type: HostIdentifier::Group,
            });
        }

        let mut group = Group::default();
        let idx = self.next_group_idx;
        let handle = GroupHandle { idx };

        group.num_instances += anonymous_instances;
        if let Some(named_instances) = named_instances {
            group.num_instances += named_instances.len();
            for (i, &instance_name) in named_instances.iter().enumerate() {
                if group.named_instances.contains_key(instance_name) {
                    return Err(HostError::DuplicateIdentifier {
                        ident: instance_name.to_owned(),
                        ident_type: HostIdentifier::GroupInstance,
                    });
                }

                let instance_handle = GroupInstanceHandle {
                    group: handle,
                    offset: i,
                };
                group
                    .named_instances
                    .insert(instance_name.to_owned(), instance_handle);
            }
        }

        self.next_group_idx += 1;
        self.groups.insert(idx, group);
        self.group_handles.insert(name.to_owned(), handle);
        Ok(handle)
    }

    pub fn create_group_joining_module<T: Module + ModuleSettings>(
        &mut self,
        group_handle: GroupHandle,
        name: &str,
        settings: T::Settings,
    ) -> HostResult<GroupJoiningModuleHandle> {
        let group = self.groups.get_mut(&group_handle.idx).unwrap();
        if group.handles.contains_key(name) {
            return Err(HostError::DuplicateIdentifier {
                ident: name.to_owned(),
                ident_type: HostIdentifier::GroupedModule,
            });
        }
        let num_args = group.num_instances;
        let module = self
            .create_variadic_module_anonymous::<T>(settings, num_args)
            .map_err(|e| HostError::GroupedModuleInit {
                group_name: self.group_name_from_handle(group_handle).to_owned(),
                module_name: name.to_owned(),
                source: e,
            })?;
        let handle = GroupJoiningModuleHandle {
            group: group_handle,
            handle: module,
        };
        let group = self.groups.get_mut(&group_handle.idx).unwrap();
        group
            .handles
            .insert(name.to_owned(), GroupedModule::Joining(handle));
        Ok(handle)
    }

    pub fn create_group_instance_variadic_module<T: Module + ModuleSettings>(
        &mut self,
        group_handle: GroupHandle,
        name: &str,
        settings: &T::Settings,
        num_args: usize,
    ) -> HostResult<GroupInstanceModuleHandle> {
        let group = self.groups.get_mut(&group_handle.idx).unwrap();
        if group.handles.contains_key(name) {
            return Err(HostError::DuplicateIdentifier {
                ident: name.to_owned(),
                ident_type: HostIdentifier::GroupedModule,
            });
        }
        let num_instances = group.num_instances;
        let modules = (0..num_instances)
            .map(|_| self.create_variadic_module_anonymous::<T>(settings.clone(), num_args))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| HostError::GroupedModuleInit {
                group_name: self.group_name_from_handle(group_handle).to_owned(),
                module_name: name.to_owned(),
                source: e,
            })?;
        let handle = GroupInstanceModuleHandle {
            group: group_handle,
            handles: modules,
        };
        let group = self.groups.get_mut(&group_handle.idx).unwrap();
        group
            .handles
            .insert(name.to_owned(), GroupedModule::Instance(handle.clone()));
        Ok(handle)
    }

    pub fn create_group_instance_module<T: Module + ModuleSettings>(
        &mut self,
        group_handle: GroupHandle,
        name: &str,
        settings: &T::Settings,
    ) -> HostResult<GroupInstanceModuleHandle> {
        self.create_group_instance_variadic_module::<T>(group_handle, name, settings, 0)
    }

    pub fn group_joining_buf<T: BufferDir>(
        &self,
        handle: GroupJoiningModuleHandle,
        name: &str,
    ) -> HostResult<GroupBufferHandle<T>> {
        Ok(self
            .variadic_buf(handle.ungrouped(), name)?
            .all(handle.group))
    }

    pub fn group_instance_buf<T: BufferDir>(
        &self,
        handle: &GroupInstanceModuleHandle,
        name: &str,
    ) -> HostResult<GroupBufferHandle<T>> {
        Ok(GroupBufferHandle {
            group: handle.group,
            handles: handle
                .handles
                .iter()
                .map(|&module| self.buf(module, name))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }

    pub fn group_instance_variadic_buf<T: BufferDir>(
        &self,
        handle: &GroupInstanceModuleHandle,
        name: &str,
    ) -> HostResult<GroupVariadicBufferHandle<T>> {
        Ok(GroupVariadicBufferHandle {
            group: handle.group,
            handles: handle
                .handles
                .iter()
                .map(|&module| self.variadic_buf(module, name))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }

    fn group_name_from_handle(&self, handle: GroupHandle) -> &str {
        self.group_handles
            .iter()
            .find(|(_, &v)| v == handle)
            .unwrap()
            .0
    }
}

#[derive(Clone)]
pub struct GroupInstanceModuleHandle {
    group: GroupHandle,
    handles: Vec<ModuleHandle>,
}

impl GroupInstanceModuleHandle {
    pub fn ungrouped(&self, instance: GroupInstanceHandle) -> HostResult<ModuleHandle> {
        if self.group != instance.group {
            Err(HostError::InstanceGroupMismatch)
        } else {
            Ok(self.handles[instance.offset])
        }
    }
}

#[derive(Clone, Copy)]
pub struct GroupJoiningModuleHandle {
    group: GroupHandle,
    handle: ModuleHandle,
}

impl GroupJoiningModuleHandle {
    pub fn ungrouped(&self) -> ModuleHandle {
        self.handle
    }
}

enum GroupedModule {
    Instance(GroupInstanceModuleHandle),
    Joining(GroupJoiningModuleHandle),
}

#[derive(Clone, Copy)]
pub struct GroupInstanceHandle {
    group: GroupHandle,
    offset: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GroupHandle {
    idx: usize,
}

#[derive(Default)]
struct Group {
    num_instances: usize,
    named_instances: FastHashMap<String, GroupInstanceHandle>,
    handles: FastHashMap<String, GroupedModule>,
}

#[derive(Clone)]
pub struct GroupBufferHandle<T: BufferDir> {
    group: GroupHandle,
    handles: Vec<ModuleBufferHandle<T>>,
}

#[derive(Clone)]
pub struct GroupVariadicBufferHandle<T: BufferDir> {
    group: GroupHandle,
    handles: Vec<ModuleVariadicBufferHandle<T>>,
}

impl<T: BufferDir> GroupVariadicBufferHandle<T> {
    pub fn at(&self, idx: usize) -> HostResult<GroupBufferHandle<T>> {
        Ok(GroupBufferHandle {
            group: self.group,
            handles: self
                .handles
                .iter()
                .map(|h| h.at(idx))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub enum BufferArity {
    Single,
    Variadic,
}

#[derive(Clone, Copy, Debug)]
pub enum BufferElemEnum {
    Signal,
    Midi,
}
impl Display for BufferElemEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BufferElemEnum::Signal => write!(f, "signal"),
            BufferElemEnum::Midi => write!(f, "MIDI"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum BufferDirEnum {
    In,
    Out,
}
impl Display for BufferDirEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BufferDirEnum::In => write!(f, "in"),
            BufferDirEnum::Out => write!(f, "out"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BufferType {
    dir: BufferDirEnum,
    elem: BufferElemEnum,
}
impl Display for BufferType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.elem, self.dir)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum HostIdentifier {
    Module,
    GroupedModule,
    Group,
    GroupInstance,
    Buffer(BufferType),
}
impl Display for HostIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostIdentifier::Module => write!(f, "module"),
            HostIdentifier::GroupedModule => write!(f, "grouped module"),
            HostIdentifier::Group => write!(f, "group"),
            HostIdentifier::GroupInstance => write!(f, "group instance"),
            HostIdentifier::Buffer(bt) => write!(f, "{}-buffer", bt),
        }
    }
}

#[derive(Error, Debug)]
pub enum ModuleError {
    #[error("{0}")]
    Custom(String),
    #[error("the {buffer_type}-buffer identifier `{ident}` already exists in this module")]
    DuplicateBufferIdentifier {
        ident: String,
        buffer_type: BufferType,
    },
}

#[derive(Error, Debug)]
pub enum HostError {
    #[error("failed to initialize module `{module_name}`")]
    ModuleInit {
        module_name: String,
        source: ModuleError,
    },
    #[error("failed to initialize module `{module_name}` in group `{group_name}`")]
    GroupedModuleInit {
        group_name: String,
        module_name: String,
        source: ModuleError,
    },
    #[error("the {ident_type} identifier `{ident}` already exists in this context")]
    DuplicateIdentifier {
        ident: String,
        ident_type: HostIdentifier,
    },
    #[error("the {ident_type} identifier `{ident}` was not found in this context")]
    NonexistentIdentifier {
        ident: String,
        ident_type: HostIdentifier,
    },
    #[error("variadic buffer index out of bounds (index: {idx}, length: {len})")]
    VariadicBufferOutOfBounds { idx: usize, len: usize },
    #[error("unexpected buffer arity (expected {expected:?}, found {found:?})")]
    UnexpectedBufferArity {
        expected: BufferArity,
        found: BufferArity,
    },
    #[error("attempted to link two grouped module buffers with mismatched groups")]
    BufferGroupMismatch,
    #[error("attempted to get a grouped module using an instance handle from a different group")]
    InstanceGroupMismatch,
}

type ModuleResult<T> = Result<T, ModuleError>;
pub type HostResult<T> = Result<T, HostError>;
