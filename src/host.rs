use std::any::Any;

use arr_macro::arr;
use rodio::Source;

use crate::{constants::*, midi::MidiEvents, output::AudioOutput, output::AudioOutputModule};

use self::private::{
    BufferInPort, BufferMarker, FastHashMap, ModuleBufferInHandle, ModuleBufferOutHandle,
    ModuleBuffersDescriptor, ModuleBuffersInInternal, ModuleBuffersOutInternal, ModuleHandle,
};

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
        BufferDir, BufferElem, BufferHandle, BufferHandleRaw, BuffersInExt, BuffersOutExt, In,
        ModuleBuffersIn, ModuleBuffersOut, ModuleDescriptor, Out, VariadicBufferHandle,
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

    #[derive(Clone, Copy, Default, PartialEq, Eq)]
    pub struct ModuleHandle {
        pub idx: usize,
    }

    #[derive(Educe, Eq)]
    #[educe(Clone, Copy, PartialEq)]
    pub struct ModuleBufferInHandle<T: BufferElem> {
        pub module_handle: ModuleHandle,
        pub buf_handle: BufferHandle<In<T>>,
    }

    #[derive(Educe, Eq)]
    #[educe(Clone, Copy, PartialEq)]
    pub struct ModuleBufferOutHandle<T: BufferElem> {
        pub module_handle: ModuleHandle,
        pub buf_handle: BufferHandle<Out<T>>,
    }

    #[derive(Clone)]
    pub struct BufferOutPort<T: BufferElem> {
        pub buffer: super::Buffer<T>,
        pub dependents: Vec<ModuleBufferInHandle<T>>,
    }

    #[derive(Clone)]
    pub enum BufferInPort<T: BufferElem> {
        OutBuffer(ModuleBufferOutHandle<T>),
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

    enum VariadicMarked<T: BufferDir> {
        Normal(BufferHandle<T>),
        Variadic(VariadicBufferHandle<T>),
    }

    pub struct BufferPorts<D: BufferDir> {
        num_args: usize,
        pub buffers: Vec<D::BufferPort>,
        handles: FastHashMap<String, VariadicMarked<D>>,
    }

    impl<D: BufferDir> BufferPorts<D> {
        fn new(descriptor: &ModuleDescriptor) -> Self {
            let mut out = Self {
                num_args: descriptor.num_args,
                buffers: Default::default(),
                handles: Default::default(),
            };
            for (marker, name, elem) in D::get_elems(descriptor).iter() {
                out.add_buffer(*marker, D::create_port(elem), name);
            }
            out
        }

        pub fn get_buf(&self, handle: BufferHandle<D>) -> &D::BufferPort {
            &self.buffers[handle.idx]
        }

        pub fn get_buf_mut(&mut self, handle: BufferHandle<D>) -> &mut D::BufferPort {
            &mut self.buffers[handle.idx]
        }

        pub fn get_handle(&self, name: &str) -> BufferHandle<D> {
            match self.handles[name] {
                VariadicMarked::Normal(handle) => handle,
                VariadicMarked::Variadic(_) => panic!(),
            }
        }

        pub fn get_variadic_handle(&self, name: &str) -> VariadicBufferHandle<D> {
            match self.handles[name] {
                VariadicMarked::Normal(_) => panic!(),
                VariadicMarked::Variadic(handle) => handle,
            }
        }

        fn add_buffer(&mut self, marker: BufferMarker, port: D::BufferPort, name: &str) {
            if self.handles.contains_key(name) {
                panic!()
            } else {
                let handle = BufferHandle::new(self.buffers.len());
                self.handles.insert(
                    name.to_owned(),
                    match marker {
                        BufferMarker::Normal => {
                            self.buffers.push(port.clone());
                            VariadicMarked::Normal(handle)
                        }
                        BufferMarker::Variadic => {
                            for _ in 0..self.num_args {
                                self.buffers.push(port.clone());
                            }
                            VariadicMarked::Variadic(VariadicBufferHandle {
                                num_args: self.num_args,
                                buffer: handle,
                            })
                        }
                    },
                );
            }
        }
    }

    #[derive(Clone, Copy)]
    pub enum BufferMarker {
        Normal,
        Variadic,
    }

    #[derive(Clone)]
    pub struct ModuleBuffersDescriptor<T: BufferElem> {
        num_args: usize,
        next_idx_buf_in: BufferHandleRaw,
        next_idx_buf_out: BufferHandleRaw,
        pub buf_in: Vec<(BufferMarker, String, T)>,
        pub buf_out: Vec<(BufferMarker, String, ())>,
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

        pub fn add_buf_in(&mut self, elem: (BufferMarker, String, T)) -> BufferHandleRaw {
            let out = self.next_idx_buf_in;
            self.next_idx_buf_in += match elem.0 {
                BufferMarker::Normal => 1,
                BufferMarker::Variadic => self.num_args,
            };
            self.buf_in.push(elem);
            out
        }

        pub fn add_buf_out(&mut self, elem: (BufferMarker, String, ())) -> BufferHandleRaw {
            let out = self.next_idx_buf_out;
            self.next_idx_buf_out += match elem.0 {
                BufferMarker::Normal => 1,
                BufferMarker::Variadic => self.num_args,
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
        pub fn new(descriptors: &ModuleDescriptor) -> Self {
            Self {
                num_dependencies: 0,
                num_finished_dependencies: AtomicUsize::new(0),
                buf_signal: BufferPorts::new(&descriptors),
                buf_midi: BufferPorts::new(&descriptors),
            }
        }
    }

    pub struct ModuleBuffersOutInternal {
        buf_signal: BufferPorts<Out<f32>>,
        buf_midi: BufferPorts<Out<crate::midi::MidiEvents>>,
    }

    impl ModuleBuffersOutInternal {
        pub fn new(descriptors: &ModuleDescriptor) -> Self {
            Self {
                buf_signal: BufferPorts::new(&descriptors),
                buf_midi: BufferPorts::new(&descriptors),
            }
        }
    }

    pub trait BufferDirSealed {
        type DescriptorElem;
        type BufferPort: Clone;
        fn get_elems(
            descriptor: &ModuleDescriptor,
        ) -> &Vec<(BufferMarker, String, Self::DescriptorElem)>;
        fn create_port(elem: &Self::DescriptorElem) -> Self::BufferPort;
    }

    impl<T: BufferElem> BufferDirSealed for In<T> {
        type DescriptorElem = T;
        type BufferPort = BufferInPort<T>;

        fn get_elems(
            descriptor: &ModuleDescriptor,
        ) -> &Vec<(BufferMarker, String, Self::DescriptorElem)> {
            &T::get_descriptor(descriptor).buf_in
        }

        fn create_port(elem: &Self::DescriptorElem) -> Self::BufferPort {
            BufferInPort::with_constant(elem.clone())
        }
    }

    impl<T: BufferElem> BufferDirSealed for Out<T> {
        type DescriptorElem = ();
        type BufferPort = BufferOutPort<T>;

        fn get_elems(
            descriptor: &ModuleDescriptor,
        ) -> &Vec<(BufferMarker, String, Self::DescriptorElem)> {
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
    pub fn at(&self, idx: usize) -> BufferHandle<T> {
        if idx >= self.num_args {
            panic!()
        }
        BufferHandle::<T>::new(self.buffer.idx + idx)
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
            BufferMarker::Normal,
            name.to_owned(),
            default,
        )))
    }

    pub fn with_buf_in<E: BufferElem>(&mut self, name: &str) -> BufferHandle<In<E>> {
        self.with_buf_in_default(name, E::default())
    }

    pub fn with_buf_out<E: BufferElem>(&mut self, name: &str) -> BufferHandle<Out<E>> {
        BufferHandle::new(E::get_descriptor_mut(self).add_buf_out((
            BufferMarker::Normal,
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
                BufferMarker::Variadic,
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
                BufferMarker::Variadic,
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
    pub fn get<T: BufferElem>(&self, buffer: BufferHandle<In<T>>) -> &Buffer<T> {
        let bufs = T::get_ext_buffers_in(&self)[buffer.idx];
        unsafe { &*bufs }
    }

    pub fn iter<T: BufferElem>(&self) -> impl Iterator<Item = &Buffer<T>> + '_ {
        T::get_ext_buffers_in(&self)
            .iter()
            .map(|&bufs| unsafe { &*bufs })
    }
}

type BuffersOutExt<T> = Vec<*mut Buffer<T>>;

pub struct ModuleBuffersOut {
    buf_signal: Vec<*mut Buffer<f32>>,
    buf_midi: Vec<*mut Buffer<MidiEvents>>,
}

impl ModuleBuffersOut {
    pub fn get<T: BufferElem>(&mut self, buffer: BufferHandle<Out<T>>) -> &mut Buffer<T> {
        let bufs = T::get_ext_buffers_out(&self)[buffer.idx];
        unsafe { &mut *bufs }
    }

    pub fn iter_mut<T: BufferElem>(&mut self) -> impl Iterator<Item = &mut Buffer<T>> + '_ {
        T::get_ext_buffers_out(self)
            .iter()
            .map(|&bufs| unsafe { &mut *bufs })
    }
}

pub trait ModuleSettings {
    type Settings;
}

pub trait Module: 'static + Any {
    fn init(descriptor: ModuleDescriptor, settings: Self::Settings) -> BuiltModuleDescriptor<Self>
    where
        Self: Sized + ModuleSettings;
    fn fill_buffers(&mut self, buffers_in: &ModuleBuffersIn, buffers_out: &mut ModuleBuffersOut);
}

struct ModuleInternals {
    module: Box<dyn Module>,
    num_args: usize,
    buf_in: ModuleBuffersInInternal,
    buf_out: ModuleBuffersOutInternal,
}

impl ModuleInternals {
    fn new<T: Module + ModuleSettings>(settings: T::Settings, num_args: usize) -> Self {
        let descriptor = T::init(ModuleDescriptor::new(num_args), settings);
        Self {
            module: descriptor.initial_data,
            num_args,
            buf_in: ModuleBuffersInInternal::new(&descriptor.buffers_descriptors),
            buf_out: ModuleBuffersOutInternal::new(&descriptor.buffers_descriptors),
        }
    }
}

pub struct Host {
    modules: FastHashMap<usize, ModuleInternals>,
    handles: FastHashMap<String, ModuleHandle>,
    next_idx: usize,
    output: rodio::source::Stoppable<AudioOutput>,
}

const OUTPUT_MODULE_NAME: &str = "audio_out";

impl Host {
    pub fn new() -> Self {
        let output = AudioOutput::new();
        let mut out = Self {
            modules: Default::default(),
            handles: Default::default(),
            next_idx: 0,
            output: output.clone().stoppable(),
        };
        out.create_module::<AudioOutputModule>(OUTPUT_MODULE_NAME, output);
        out
    }

    pub fn create_variadic_module<T: Module + ModuleSettings>(
        &mut self,
        name: &str,
        settings: T::Settings,
        num_args: usize,
    ) {
        if self.handles.contains_key(name) {
            panic!()
        }
        let module = ModuleInternals::new::<T>(settings, num_args);
        let idx = self.next_idx;
        self.next_idx += 1;
        self.modules.insert(idx, module);
        self.handles.insert(name.to_owned(), ModuleHandle { idx });
    }

    pub fn create_module<T: Module + ModuleSettings>(&mut self, name: &str, settings: T::Settings) {
        self.create_variadic_module::<T>(name, settings, 0)
    }

    fn set_buffer_in<T: BufferElem>(
        &mut self,
        port_handle: ModuleBufferInHandle<T>,
        new: BufferInPort<T>,
    ) {
        let module_in = self
            .modules
            .get_mut(&port_handle.module_handle.idx)
            .unwrap();

        let port = &T::get_buffers_in(&module_in.buf_in).get_buf(port_handle.buf_handle);
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
        module_out: &str,
        buf_name_out: &str,
        module_in: &str,
        buf_name_in: &str,
    ) {
        let handle_out = self.handles[module_out];
        let buf_out = ModuleBufferOutHandle {
            module_handle: handle_out,
            buf_handle: T::get_buffers_out(&self.modules[&handle_out.idx].buf_out)
                .get_handle(buf_name_out),
        };

        let handle_in = self.handles[module_in];
        let buf_in = ModuleBufferInHandle {
            module_handle: handle_in,
            buf_handle: T::get_buffers_in(&self.modules[&handle_in.idx].buf_in)
                .get_handle(buf_name_in),
        };

        self.set_buffer_in(buf_in, BufferInPort::OutBuffer(buf_out));
    }

    pub fn link_value<T: BufferElem>(&mut self, value: T, module_in: &str, buf_name_in: &str) {
        let handle_in = self.handles[module_in];
        let buf_in = ModuleBufferInHandle {
            module_handle: handle_in,
            buf_handle: T::get_buffers_in(&self.modules[&handle_in.idx].buf_in)
                .get_handle(buf_name_in),
        };

        self.set_buffer_in(buf_in, BufferInPort::with_constant(value));
    }

    pub fn destroy_module(&mut self, module: &str) {
        fn remove_dependents<T: BufferElem>(host: &mut Host, module: &ModuleInternals) {
            for out_port in T::get_buffers_out(&module.buf_out).buffers.iter() {
                for dep_handle in out_port.dependents.iter() {
                    host.set_buffer_in(dep_handle.clone(), BufferInPort::default());
                }
            }
        }

        fn remove_dependencies<T: BufferElem>(
            host: &mut Host,
            module: &ModuleInternals,
            module_handle: ModuleHandle,
        ) {
            for (port_idx, in_port) in T::get_buffers_in(&module.buf_in).buffers.iter().enumerate()
            {
                match in_port {
                    BufferInPort::OutBuffer(_) => {
                        host.set_buffer_in(
                            ModuleBufferInHandle {
                                module_handle: module_handle.clone(),
                                buf_handle: BufferHandle::new(port_idx),
                            },
                            BufferInPort::<T>::default(),
                        );
                    }
                    BufferInPort::Constant(_) => {}
                }
            }
        }

        if module == OUTPUT_MODULE_NAME {
            panic!()
        }

        let handle = self.handles.remove(module).unwrap();

        let module = self.modules.remove(&handle.idx).unwrap();
        remove_dependents::<f32>(self, &module);
        remove_dependencies::<f32>(self, &module, handle);
        remove_dependents::<MidiEvents>(self, &module);
        remove_dependencies::<MidiEvents>(self, &module, handle);
    }

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
                        let buf_out = &host.modules.get(&handle.module_handle.idx).unwrap().buf_out;
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
}
